//! `index.json`: every published release, each signed on its own.
//!
//! Releases are signed one by one, so the registry CI can add a release without re-signing the
//! others, and a client can use the releases that verify even if one entry is damaged: each
//! entry is parsed on its own (bugs-1 #12). The index as a whole, its time and the set of
//! releases, is covered by the signed snapshot ([`crate::Snapshot`], security-2 S6), so a mirror
//! cannot withhold a release or replay an old index.

use std::collections::{BTreeMap, BTreeSet};

use semver::Version;
use serde::{Deserialize, Serialize};

use crate::{
    Canonical, Document, Error, ExtensionId, KeyRole, Keyset, Sha256Digest, Signed, Tier,
    Timestamp, VerifiedSnapshot,
    canonical::CanonicalWriter,
    check_schema, parse_json,
    text::{check_name, check_text},
};

pub const INDEX_SCHEMA: u32 = 1;

/// Decision 12: a `.fyxext` package is at most 50 MiB.
pub const MAX_PACKAGE_SIZE: u64 = 50 * 1024 * 1024;

/// The most `index.json` may weigh; a client refuses a larger download before parsing it.
pub const MAX_INDEX_SIZE: u64 = 32 * 1024 * 1024;
/// The most entries `index.json` may list.
pub const MAX_RELEASES: usize = 100_000;

const RELEASE_DOMAIN: &str = "fyx-registry/release/v1";
const MAX_NAME_LENGTH: usize = 64;
const MAX_DESCRIPTION_LENGTH: usize = 300;
const MAX_GITHUB_LOGIN_LENGTH: usize = 39;
const MAX_PROCESS_NAME_LENGTH: usize = 64;
const MAX_DOMAIN_LENGTH: usize = 253;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawIndex")]
pub struct Index {
    pub schema: u32,
    /// Unsigned here; [`Index::verify_with_snapshot`] takes the time from the signed snapshot.
    pub generated_at: Timestamp,
    pub releases: Vec<Signed<Release>>,
    /// Entries that did not parse, kept so the rest of the index stays usable. Never written
    /// back: the registry CI refuses an index that has any ([`Index::verify_strict`]).
    #[serde(skip_serializing)]
    pub malformed: Vec<MalformedRelease>,
}

/// An entry of `index.json` that is not a signed release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MalformedRelease {
    /// Position in `releases`, from 0.
    pub position: usize,
    /// `signed.id` and `signed.version` as written, if they are strings, for the message.
    pub id: Option<String>,
    pub version: Option<String>,
    pub error: Error,
}

/// The index as read, before each entry is parsed on its own.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawIndex {
    schema: u32,
    generated_at: Timestamp,
    releases: Vec<serde_json::Value>,
}

impl TryFrom<RawIndex> for Index {
    type Error = String;

    fn try_from(raw: RawIndex) -> Result<Self, String> {
        if raw.releases.len() > MAX_RELEASES {
            return Err(format!(
                "the index lists {} releases, the cap is {MAX_RELEASES}",
                raw.releases.len()
            ));
        }
        let mut releases = Vec::with_capacity(raw.releases.len());
        let mut malformed = Vec::new();
        for (position, value) in raw.releases.into_iter().enumerate() {
            let field = |name: &str| {
                value
                    .pointer(&format!("/signed/{name}"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            };
            let (id, version) = (field("id"), field("version"));
            match serde_json::from_value::<Signed<Release>>(value) {
                Ok(release) => releases.push(release),
                Err(error) => malformed.push(MalformedRelease {
                    position,
                    id,
                    version,
                    error: Error::Parse {
                        document: Document::Release,
                        message: error.to_string(),
                    },
                }),
            }
        }
        Ok(Self {
            schema: raw.schema,
            generated_at: raw.generated_at,
            releases,
            malformed,
        })
    }
}

/// One published version of one extension. Every field is covered by the release signature.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Release {
    pub id: ExtensionId,
    /// `major.minor.patch`; pre-release and build suffixes are refused in schema 1.
    pub version: Version,
    pub name: String,
    pub description: String,
    /// GitHub logins of the extension's owners (decision 14), shown as the publisher.
    pub owners: BTreeSet<String>,
    pub source: Source,
    pub package: Package,
    /// When the registry CI published the release; the 24 h age gate counts from here.
    pub published_at: Timestamp,
    /// A security fix that skips the age gate (decision 17).
    pub urgent: bool,
    /// The highest tier among the capabilities, as declared in `extension.toml`.
    pub tier: Tier,
    pub capabilities: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub network_domains: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub processes: BTreeSet<String>,
    /// The package has a Zed `extension.wasm` ([`crate::runs_zed_extension`]): it also runs as a
    /// Zed extension, with unrestricted network and process access. Such a release is T2
    /// whatever it declares (owner's Q1), and the host shows
    /// [`crate::RUNS_ZED_EXTENSION_WARNING`]. [`Release::verify_package`] checks it against the
    /// package. A client older than this field reads such a release as malformed and skips it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub runs_zed_extension: bool,
}

/// Where the registry CI built the package from (decision 15).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    /// `https://` URL of the source repository.
    pub repository: String,
    /// Full 40-digit lowercase commit SHA.
    pub commit: String,
    /// Directory of the extension inside the repository, `.` for the root.
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Package {
    /// `https://` URL of the `.fyxext` file.
    pub url: String,
    pub sha256: Sha256Digest,
    /// Size in bytes, so a client can refuse an oversized download before hashing it.
    pub size: u64,
}

impl Canonical for Release {
    const ROLE: KeyRole = KeyRole::Release;

    fn canonical_bytes(&self) -> Vec<u8> {
        let mut writer = CanonicalWriter::new(RELEASE_DOMAIN);
        writer.string(self.id.as_str());
        writer.string(&self.version.to_string());
        writer.string(&self.name);
        writer.string(&self.description);
        writer.strings(self.owners.iter().map(String::as_str));
        writer.string(&self.source.repository);
        writer.string(&self.source.commit);
        writer.string(&self.source.path);
        writer.string(&self.package.url);
        writer.byte_string(&self.package.sha256.0);
        writer.u64(self.package.size);
        writer.u64(self.published_at.0);
        writer.bool(self.urgent);
        writer.string(self.tier.as_str());
        writer.strings(self.capabilities.iter().map(String::as_str));
        writer.strings(self.network_domains.iter().map(String::as_str));
        writer.strings(self.processes.iter().map(String::as_str));
        // Written only when set, so every release signed before the field keeps its signature.
        if self.runs_zed_extension {
            writer.bool(true);
        }
        writer.finish()
    }
}

impl Release {
    /// Checks everything the schema cannot express. A release that fails is never installed.
    pub fn validate(&self) -> Result<(), Error> {
        let invalid =
            |message: String| Error::Invalid(format!("{}@{}: {message}", self.id, self.version));

        if !self.version.pre.is_empty() || !self.version.build.is_empty() {
            return Err(invalid(
                "the version must be major.minor.patch without a suffix".into(),
            ));
        }
        check_name("name", &self.name, MAX_NAME_LENGTH).map_err(invalid)?;
        check_text("description", &self.description, 0, MAX_DESCRIPTION_LENGTH).map_err(invalid)?;
        if self.owners.is_empty() {
            return Err(invalid("at least one owner is required".into()));
        }
        for owner in &self.owners {
            if !is_github_login(owner) {
                return Err(invalid(format!("{owner:?} is not a GitHub login")));
            }
        }

        check_https_url("source.repository", &self.source.repository).map_err(invalid)?;
        if self.source.commit.len() != 40
            || !self
                .source
                .commit
                .chars()
                .all(|character| matches!(character, '0'..='9' | 'a'..='f'))
        {
            return Err(invalid(
                "source.commit must be a full 40-digit lowercase commit SHA".into(),
            ));
        }
        if !is_relative_path(&self.source.path) {
            return Err(invalid(format!(
                "source.path {:?} must be a relative path without '..' (use '.' for the root)",
                self.source.path
            )));
        }

        check_https_url("package.url", &self.package.url).map_err(invalid)?;
        if self.package.size == 0 || self.package.size > MAX_PACKAGE_SIZE {
            return Err(invalid(format!(
                "package.size {} must be between 1 and {MAX_PACKAGE_SIZE} bytes",
                self.package.size
            )));
        }

        for capability in &self.capabilities {
            if !is_capability_name(capability) {
                return Err(invalid(format!(
                    "{capability:?} is not a capability name like \"ui:panel\""
                )));
            }
            // Capabilities from a newer Fyx are allowed here; the update planner never grants
            // them silently.
            if let Some(tier) = Tier::of_capability(capability)
                && tier > self.tier
            {
                return Err(invalid(format!(
                    "{capability} is tier {tier}, above the declared tier {}",
                    self.tier
                )));
            }
        }
        if self.runs_zed_extension && self.tier != Tier::T2 {
            return Err(invalid(
                "a release that runs a Zed extension is T2, whatever it declares".into(),
            ));
        }
        if !self.network_domains.is_empty() && !self.capabilities.contains("network:fetch") {
            return Err(invalid(
                "network_domains need the network:fetch capability".into(),
            ));
        }
        for domain in &self.network_domains {
            if !is_domain_pattern(domain) {
                return Err(invalid(format!(
                    "{domain:?} is not a domain like \"example.com\" or \"*.example.com\" (no \
                     single labels, IP addresses, localhost or public suffixes)"
                )));
            }
        }
        if !self.processes.is_empty() && !self.capabilities.contains("process:spawn") {
            return Err(invalid(
                "processes need the process:spawn capability".into(),
            ));
        }
        for process in &self.processes {
            if !is_process_name(process) {
                return Err(invalid(format!(
                    "{process:?} is not a program name (no paths, at most \
                     {MAX_PROCESS_NAME_LENGTH} characters)"
                )));
            }
        }
        Ok(())
    }

    /// Checks downloaded package bytes against the signed size and hash, and that the package
    /// has a Zed `extension.wasm` exactly when the release says so.
    pub fn verify_package(&self, package: &[u8]) -> Result<(), Error> {
        let size = package.len() as u64;
        if size != self.package.size {
            return Err(Error::PackageMismatch(format!(
                "{}@{} is {size} bytes, the index says {}",
                self.id, self.version, self.package.size
            )));
        }
        let digest = Sha256Digest::of(package);
        if digest != self.package.sha256 {
            return Err(Error::PackageMismatch(format!(
                "{}@{} has sha256 {digest}, the index says {}",
                self.id, self.version, self.package.sha256
            )));
        }
        let runs_zed_extension = crate::runs_zed_extension(package);
        if runs_zed_extension != self.runs_zed_extension {
            return Err(Error::PackageMismatch(format!(
                "{}@{} {} a Zed extension.wasm, the release says {}",
                self.id,
                self.version,
                if runs_zed_extension { "has" } else { "has no" },
                if self.runs_zed_extension {
                    "it has one"
                } else {
                    "it has none"
                }
            )));
        }
        Ok(())
    }

    pub fn from_json(bytes: &[u8]) -> Result<Signed<Release>, Error> {
        parse_json(bytes, Document::Release)
    }
}

impl Signed<Release> {
    /// The signature is checked before the content, so a tampered release reports a bad
    /// signature rather than whatever field the tampering broke.
    pub fn verify_release(&self, keyset: &Keyset) -> Result<&Release, Error> {
        self.verify(keyset)?;
        self.signed.validate()?;
        Ok(&self.signed)
    }
}

/// An entry that was left out of a [`VerifiedIndex`], and why. `id` and `version` are `None`
/// for an entry too damaged to name them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RejectedRelease {
    pub id: Option<String>,
    pub version: Option<String>,
    pub error: Error,
}

impl RejectedRelease {
    /// `id@version`, or what is known of it.
    pub fn label(&self) -> String {
        format!(
            "{}@{}",
            self.id.as_deref().unwrap_or("<unknown>"),
            self.version.as_deref().unwrap_or("<unknown>")
        )
    }
}

/// The releases of an index whose signatures and contents check out.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VerifiedIndex {
    generated_at: Timestamp,
    releases: BTreeMap<ExtensionId, BTreeMap<Version, Release>>,
    rejected: Vec<RejectedRelease>,
}

impl VerifiedIndex {
    pub fn generated_at(&self) -> Timestamp {
        self.generated_at
    }

    pub fn extensions(&self) -> impl Iterator<Item = &ExtensionId> {
        self.releases.keys()
    }

    /// Releases of `id`, oldest version first.
    pub fn releases(&self, id: &ExtensionId) -> impl DoubleEndedIterator<Item = &Release> {
        self.releases.get(id).into_iter().flat_map(BTreeMap::values)
    }

    pub fn release(&self, id: &ExtensionId, version: &Version) -> Option<&Release> {
        self.releases.get(id)?.get(version)
    }

    pub fn latest(&self, id: &ExtensionId) -> Option<&Release> {
        self.releases(id).next_back()
    }

    pub fn rejected(&self) -> &[RejectedRelease] {
        &self.rejected
    }
}

impl Index {
    pub fn new(generated_at: Timestamp) -> Self {
        Self {
            schema: INDEX_SCHEMA,
            generated_at,
            releases: Vec::new(),
            malformed: Vec::new(),
        }
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() as u64 > MAX_INDEX_SIZE {
            return Err(Error::Invalid(format!(
                "index.json has {} bytes, the cap is {MAX_INDEX_SIZE}",
                bytes.len()
            )));
        }
        check_schema(bytes, Document::Index, "/schema", INDEX_SCHEMA)?;
        parse_json(bytes, Document::Index)
    }

    /// The client's path: `bytes` must be the index the snapshot names (size and SHA-256), the
    /// releases are checked against the snapshot's keyset (revoked keys removed), and the time
    /// is the snapshot's signed `generated_at`. `keyset` is the client's trusted keyset.
    pub fn verify_with_snapshot(
        bytes: &[u8],
        snapshot: &VerifiedSnapshot,
        keyset: &Keyset,
    ) -> Result<VerifiedIndex, Error> {
        snapshot.check_index(bytes)?;
        let index = Self::from_json(bytes)?;
        let keyset = snapshot.effective_keyset(keyset)?;
        let mut verified = index.verify(&keyset);
        verified.generated_at = snapshot.generated_at();
        Ok(verified)
    }

    pub fn to_json(&self) -> Result<Vec<u8>, Error> {
        to_pretty_json(self, Document::Index)
    }

    /// Keeps every release that verifies; the others, and the malformed entries, are listed in
    /// [`VerifiedIndex::rejected`]. Duplicates are counted among verified releases only, so a
    /// forged copy cannot evict the genuine one (security-2 S11): identical copies count once,
    /// and a version with two different valid copies is rejected in both, since the client
    /// cannot know which one the registry meant.
    pub fn verify(&self, keyset: &Keyset) -> VerifiedIndex {
        let mut verified = VerifiedIndex {
            generated_at: self.generated_at,
            ..VerifiedIndex::default()
        };
        for malformed in &self.malformed {
            verified.rejected.push(RejectedRelease {
                id: malformed.id.clone(),
                version: malformed.version.clone(),
                error: malformed.error.clone(),
            });
        }
        let mut valid = BTreeMap::<(&ExtensionId, &Version), Vec<&Release>>::new();
        for release in &self.releases {
            match release.verify_release(keyset) {
                Ok(release) => valid
                    .entry((&release.id, &release.version))
                    .or_default()
                    .push(release),
                Err(error) => verified.rejected.push(RejectedRelease {
                    id: Some(release.signed.id.to_string()),
                    version: Some(release.signed.version.to_string()),
                    error,
                }),
            }
        }
        for ((id, version), copies) in valid {
            if copies.iter().all(|copy| *copy == copies[0]) {
                verified
                    .releases
                    .entry(id.clone())
                    .or_default()
                    .insert(version.clone(), copies[0].clone());
            } else {
                for _ in &copies {
                    verified.rejected.push(RejectedRelease {
                        id: Some(id.to_string()),
                        version: Some(version.to_string()),
                        error: Error::Invalid(format!(
                            "{id}@{version} appears {} times in the index with different contents",
                            copies.len()
                        )),
                    });
                }
            }
        }
        verified
    }

    /// For the registry CI: the whole index must verify, with no duplicate entries at all.
    pub fn verify_strict(&self, keyset: &Keyset) -> Result<VerifiedIndex, Error> {
        let verified = self.verify(keyset);
        if let Some(rejected) = verified.rejected.first() {
            return Err(rejected.error.clone());
        }
        let listed = self.releases.len();
        let distinct: usize = verified.releases.values().map(BTreeMap::len).sum();
        if distinct != listed {
            return Err(Error::Invalid(format!(
                "the index lists {listed} releases but only {distinct} distinct versions"
            )));
        }
        Ok(verified)
    }

    /// Adds a verified release, keeping the index sorted by id and version. Refuses a version
    /// that is already published: a published release is never replaced, only blocked.
    pub fn add(
        &mut self,
        release: Signed<Release>,
        keyset: &Keyset,
        generated_at: Timestamp,
    ) -> Result<(), Error> {
        self.verify_strict(keyset)?;
        release.verify_release(keyset)?;
        // A linear scan rather than a binary search: a hand-edited index may not be sorted.
        if self.releases.iter().any(|existing| {
            existing.signed.id == release.signed.id
                && existing.signed.version == release.signed.version
        }) {
            return Err(Error::Invalid(format!(
                "{}@{} is already in the index",
                release.signed.id, release.signed.version
            )));
        }
        self.releases.push(release);
        self.sort();
        self.generated_at = generated_at;
        Ok(())
    }

    /// Sorts releases by id and version, the order [`Index::add`] writes.
    pub fn sort(&mut self) {
        self.releases.sort_by(|left, right| {
            (&left.signed.id, &left.signed.version).cmp(&(&right.signed.id, &right.signed.version))
        });
    }
}

pub(crate) fn to_pretty_json<T: Serialize>(
    value: &T,
    document: Document,
) -> Result<Vec<u8>, Error> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|error| Error::Parse {
        document,
        message: error.to_string(),
    })?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn check_https_url(field: &str, value: &str) -> Result<(), String> {
    let valid = value
        .strip_prefix("https://")
        .is_some_and(|rest| !rest.is_empty() && !rest.starts_with('/'))
        && !value
            .chars()
            .any(|character| character.is_whitespace() || character.is_control());
    if valid {
        Ok(())
    } else {
        Err(format!("{field} {value:?} must be an https:// URL"))
    }
}

fn is_github_login(login: &str) -> bool {
    !login.is_empty()
        && login.len() <= MAX_GITHUB_LOGIN_LENGTH
        && !login.starts_with('-')
        && !login.ends_with('-')
        && login
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
}

fn is_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.chars().any(char::is_control)
        && path
            .split('/')
            .all(|component| !component.is_empty() && component != "..")
}

fn is_name_part(part: &str) -> bool {
    part.starts_with(|character: char| character.is_ascii_lowercase())
        && part
            .chars()
            .all(|character| matches!(character, 'a'..='z' | '0'..='9' | '-'))
}

fn is_capability_name(capability: &str) -> bool {
    capability
        .split_once(':')
        .is_some_and(|(namespace, name)| is_name_part(namespace) && is_name_part(name))
}

/// A domain or `*.domain` that names one site: at least two labels, not all-numeric (no IP
/// literals), not `localhost`, and neither the domain nor a wildcard's base a public suffix such
/// as `com` or `co.uk` (security-2 S14).
fn is_domain_pattern(pattern: &str) -> bool {
    let domain = pattern.strip_prefix("*.").unwrap_or(pattern);
    let labels: Vec<&str> = domain.split('.').collect();
    let names_one_site = labels.len() >= 2
        && !labels.contains(&"localhost")
        && !labels
            .last()
            .is_some_and(|label| label.chars().all(|character| character.is_ascii_digit()))
        && !crate::text::is_public_suffix(domain);
    names_one_site
        && domain.len() <= MAX_DOMAIN_LENGTH
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .chars()
                    .all(|character| matches!(character, 'a'..='z' | '0'..='9' | '-'))
        })
}

fn is_process_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_PROCESS_NAME_LENGTH
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._+-".contains(character))
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::canonical::CanonicalReader;

    pub(crate) fn release(id: &str, version: &str) -> Release {
        Release {
            id: ExtensionId::new(id).expect("valid id"),
            version: Version::parse(version).expect("valid version"),
            name: "Todo".into(),
            description: "Keeps a list of things to do.".into(),
            owners: ["alice".to_string()].into(),
            source: Source {
                repository: "https://github.com/alice/todo".into(),
                commit: "0123456789abcdef0123456789abcdef01234567".into(),
                path: ".".into(),
            },
            package: Package {
                url: format!("https://example.invalid/{id}-{version}.fyxext"),
                sha256: Sha256Digest::of(b"package"),
                size: 7,
            },
            published_at: Timestamp(1_000_000),
            urgent: false,
            tier: Tier::T0,
            capabilities: ["ui:panel".to_string()].into(),
            network_domains: BTreeSet::new(),
            processes: BTreeSet::new(),
            runs_zed_extension: false,
        }
    }

    /// Rebuilds a release from its canonical bytes; see `CanonicalReader`.
    pub(crate) fn decode_release(bytes: &[u8]) -> Option<Release> {
        let mut reader = CanonicalReader::new(bytes, RELEASE_DOMAIN)?;
        let release = Release {
            id: ExtensionId::new(reader.string()?).ok()?,
            version: Version::parse(&reader.string()?).ok()?,
            name: reader.string()?,
            description: reader.string()?,
            owners: reader.strings()?.into_iter().collect(),
            source: Source {
                repository: reader.string()?,
                commit: reader.string()?,
                path: reader.string()?,
            },
            package: Package {
                url: reader.string()?,
                sha256: Sha256Digest(reader.byte_string()?.try_into().ok()?),
                size: reader.u64()?,
            },
            published_at: Timestamp(reader.u64()?),
            urgent: reader.bool()?,
            tier: match reader.string()?.as_str() {
                "t0" => Tier::T0,
                "t1" => Tier::T1,
                "t2" => Tier::T2,
                _ => return None,
            },
            capabilities: reader.strings()?.into_iter().collect(),
            network_domains: reader.strings()?.into_iter().collect(),
            processes: reader.strings()?.into_iter().collect(),
            // Only ever written as `true`; a written `false` is not canonical.
            runs_zed_extension: if reader.is_empty() {
                false
            } else if reader.bool()? {
                true
            } else {
                return None;
            },
        };
        reader.finish()?;
        Some(release)
    }
}

#[cfg(test)]
mod tests {
    use super::{test_support::*, *};
    use crate::signing::test_keys::{keyset, registry_key, rotation_key, signing_key};
    use proptest::prelude::*;

    fn signed(release: Release) -> Signed<Release> {
        Signed::sign(release, &registry_key())
    }

    #[test]
    fn sample_release_is_valid() {
        assert_eq!(release("todo", "1.0.0").validate(), Ok(()));
    }

    #[test]
    fn validation_refuses_bad_fields() {
        let cases: Vec<(&str, Box<dyn Fn(&mut Release)>)> = vec![
            (
                "pre-release",
                Box::new(|r| r.version = Version::parse("1.0.0-beta").expect("v")),
            ),
            (
                "build suffix",
                Box::new(|r| r.version = Version::parse("1.0.0+1").expect("v")),
            ),
            ("empty name", Box::new(|r| r.name.clear())),
            (
                "control in name",
                Box::new(|r| r.name = "a\u{1b}[31m".into()),
            ),
            (
                "bidi override in name",
                Box::new(|r| r.name = "Todo\u{202e}gpj".into()),
            ),
            (
                "zero width in description",
                Box::new(|r| r.description = "Safe\u{200b}ly".into()),
            ),
            (
                "line separator in description",
                Box::new(|r| r.description = "One\u{2028}Two".into()),
            ),
            (
                "symbol in name",
                Box::new(|r| r.name = "Todo \u{2714}".into()),
            ),
            ("padded name", Box::new(|r| r.name = " Todo".into())),
            (
                "long description",
                Box::new(|r| r.description = "x".repeat(301)),
            ),
            ("no owners", Box::new(|r| r.owners.clear())),
            (
                "bad owner",
                Box::new(|r| {
                    r.owners.insert("-bob".into());
                }),
            ),
            (
                "http source",
                Box::new(|r| r.source.repository = "http://github.com/a/b".into()),
            ),
            (
                "short commit",
                Box::new(|r| r.source.commit = "0123abc".into()),
            ),
            (
                "upper commit",
                Box::new(|r| r.source.commit = r.source.commit.to_uppercase()),
            ),
            ("absolute path", Box::new(|r| r.source.path = "/etc".into())),
            (
                "parent path",
                Box::new(|r| r.source.path = "a/../../b".into()),
            ),
            ("empty path", Box::new(|r| r.source.path = String::new())),
            (
                "http package",
                Box::new(|r| r.package.url = "http://x/y.fyxext".into()),
            ),
            ("empty package", Box::new(|r| r.package.size = 0)),
            (
                "huge package",
                Box::new(|r| r.package.size = MAX_PACKAGE_SIZE + 1),
            ),
            (
                "bad capability",
                Box::new(|r| {
                    r.capabilities.insert("Panel".into());
                }),
            ),
            (
                "tier too low",
                Box::new(|r| {
                    r.capabilities.insert("workspace:read-files".into());
                }),
            ),
            (
                "domains without fetch",
                Box::new(|r| {
                    r.network_domains.insert("example.com".into());
                }),
            ),
            (
                "processes without spawn",
                Box::new(|r| {
                    r.processes.insert("git".into());
                }),
            ),
            (
                "bad domain",
                Box::new(|r| {
                    r.tier = Tier::T2;
                    r.capabilities.insert("network:fetch".into());
                    r.network_domains.insert("https://example.com".into());
                }),
            ),
            (
                "process path",
                Box::new(|r| {
                    r.tier = Tier::T2;
                    r.capabilities.insert("process:spawn".into());
                    r.processes.insert("/usr/bin/git".into());
                }),
            ),
        ];
        for (name, mutate) in cases {
            let mut release = release("todo", "1.0.0");
            mutate(&mut release);
            assert!(release.validate().is_err(), "{name} should be refused");
        }
    }

    #[test]
    fn validation_accepts_t2_with_declared_domains_and_unknown_capabilities() {
        let mut release = release("todo", "1.0.0");
        release.tier = Tier::T2;
        release.capabilities.extend([
            "network:fetch".to_string(),
            "process:spawn".to_string(),
            "ui:from-the-future".to_string(),
        ]);
        release.network_domains.extend([
            "example.com".to_string(),
            "*.example.org".to_string(),
            "api.example.co.uk".to_string(),
        ]);
        release
            .processes
            .extend(["git".to_string(), "clang++".to_string()]);
        assert_eq!(release.validate(), Ok(()));
    }

    #[test]
    fn index_json_round_trips() {
        let mut index = Index::new(Timestamp(5));
        index.releases.push(signed(release("todo", "1.0.0")));
        let json = index.to_json().expect("serializes");
        assert_eq!(Index::from_json(&json), Ok(index));
        assert!(
            !String::from_utf8(json)
                .expect("utf-8")
                .contains("malformed")
        );
    }

    #[test]
    fn index_refuses_newer_schema_and_unknown_fields() {
        assert!(matches!(
            Index::from_json(br#"{"schema": 2, "whatever": true}"#),
            Err(Error::UnsupportedSchema { found: 2, .. })
        ));
        assert!(matches!(
            Index::from_json(br#"{"schema": 1, "generated_at": 0, "releases": [], "x": 1}"#),
            Err(Error::Parse { .. })
        ));
    }

    #[test]
    fn verify_keeps_good_releases_and_lists_bad_ones() {
        let mut tampered = signed(release("todo", "1.1.0"));
        tampered.signed.package.sha256 = Sha256Digest::of(b"other package");
        let stranger = Signed::sign(release("todo", "1.2.0"), &signing_key("stranger", 9));
        let mut invalid = release("todo", "1.3.0");
        invalid.source.commit = "nope".into();

        let mut index = Index::new(Timestamp(0));
        index.releases = vec![
            signed(release("todo", "1.0.0")),
            tampered,
            stranger,
            signed(invalid),
            Signed::sign(release("lint", "0.1.0"), &rotation_key()),
            signed(release("dup", "1.0.0")),
            signed(release("dup", "1.0.0")),
            signed(release("conflict", "1.0.0")),
            signed({
                let mut other = release("conflict", "1.0.0");
                other.description = "Another description.".into();
                other
            }),
        ];
        let verified = index.verify(&keyset());
        let todo = ExtensionId::new("todo").expect("valid");
        assert_eq!(
            verified
                .releases(&todo)
                .map(|release| release.version.to_string())
                .collect::<Vec<_>>(),
            ["1.0.0"]
        );
        assert!(
            verified
                .latest(&ExtensionId::new("lint").expect("valid"))
                .is_some()
        );
        // Identical copies count once; different valid copies are both rejected.
        assert!(
            verified
                .latest(&ExtensionId::new("dup").expect("valid"))
                .is_some()
        );
        assert!(
            verified
                .latest(&ExtensionId::new("conflict").expect("valid"))
                .is_none()
        );

        let rejected: Vec<_> = verified
            .rejected()
            .iter()
            .map(|rejected| (rejected.label(), rejected.error.clone()))
            .collect();
        assert!(matches!(&rejected[0], (v, Error::BadSignature { .. }) if v == "todo@1.1.0"));
        assert!(matches!(&rejected[1], (v, Error::UnknownKey { .. }) if v == "todo@1.2.0"));
        assert!(matches!(&rejected[2], (v, Error::Invalid(_)) if v == "todo@1.3.0"));
        assert_eq!(rejected.len(), 5);
        assert!(index.verify_strict(&keyset()).is_err());

        // The CI also refuses identical duplicates.
        let mut duplicated = Index::new(Timestamp(0));
        duplicated.releases = vec![
            signed(release("dup", "1.0.0")),
            signed(release("dup", "1.0.0")),
        ];
        assert!(duplicated.verify_strict(&keyset()).is_err());
    }

    /// bugs-1 #12: one damaged entry no longer discards the index.
    #[test]
    fn a_malformed_entry_is_rejected_alone() {
        let mut index = Index::new(Timestamp(0));
        index.releases.push(signed(release("todo", "1.0.0")));
        let mut json: serde_json::Value =
            serde_json::from_slice(&index.to_json().expect("serializes")).expect("json");
        let mut broken = json["releases"][0].clone();
        broken["signed"]["version"] = "2.0.0".into();
        broken["signed"]["package"]["sha256"] = "BC4A".into();
        json["releases"].as_array_mut().expect("array").push(broken);
        json["releases"]
            .as_array_mut()
            .expect("array")
            .push(serde_json::json!(17));
        let bytes = serde_json::to_vec(&json).expect("serializes");

        let index = Index::from_json(&bytes).expect("the index still parses");
        let verified = index.verify(&keyset());
        let todo = ExtensionId::new("todo").expect("valid");
        assert!(verified.release(&todo, &Version::new(1, 0, 0)).is_some());
        let labels: Vec<String> = verified
            .rejected()
            .iter()
            .map(RejectedRelease::label)
            .collect();
        assert_eq!(labels, ["todo@2.0.0", "<unknown>@<unknown>"]);
        assert!(index.verify_strict(&keyset()).is_err());
    }

    #[test]
    fn domains_name_one_site() {
        for good in [
            "example.com",
            "*.example.org",
            "api.example.co.uk",
            "a1.b-c.io",
        ] {
            assert!(is_domain_pattern(good), "{good}");
        }
        for bad in [
            "com",
            "*.com",
            "*.co.uk",
            "co.uk",
            "*.github.io",
            "localhost",
            "*.localhost",
            "dev.localhost",
            "127.0.0.1",
            "*.0.1",
            "example",
            "",
            "*.",
        ] {
            assert!(!is_domain_pattern(bad), "{bad}");
        }
    }

    #[test]
    fn add_keeps_order_and_refuses_republishing() {
        let keyset = keyset();
        let mut index = Index::new(Timestamp(0));
        for (id, version) in [("todo", "1.1.0"), ("lint", "2.0.0"), ("todo", "1.0.0")] {
            index
                .add(signed(release(id, version)), &keyset, Timestamp(9))
                .expect("adds");
        }
        let order: Vec<_> = index
            .releases
            .iter()
            .map(|release| format!("{}@{}", release.signed.id, release.signed.version))
            .collect();
        assert_eq!(order, ["lint@2.0.0", "todo@1.0.0", "todo@1.1.0"]);
        assert_eq!(index.generated_at, Timestamp(9));

        let mut republished = release("todo", "1.0.0");
        republished.package.sha256 = Sha256Digest::of(b"different");
        assert!(
            index
                .add(signed(republished), &keyset, Timestamp(10))
                .is_err()
        );

        let mut tampered = signed(release("todo", "1.2.0"));
        tampered.signed.urgent = true;
        assert!(index.add(tampered, &keyset, Timestamp(10)).is_err());
        assert_eq!(index.releases.len(), 3);

        // A hand-edited, unsorted index still catches the duplicate.
        index.releases.reverse();
        assert!(
            index
                .add(signed(release("lint", "2.0.0")), &keyset, Timestamp(10))
                .is_err()
        );
    }

    /// Owner's Q1 (revised): a package with a Zed `extension.wasm` is accepted, but its release
    /// must say so, and is T2.
    #[test]
    fn a_zed_extension_release_is_declared_and_t2() {
        let mut release = release("todo", "1.0.0");
        let plain = release.canonical_bytes();
        release.runs_zed_extension = true;
        assert_ne!(release.canonical_bytes(), plain, "the flag is signed");
        release.runs_zed_extension = false;
        assert_eq!(
            release.canonical_bytes(),
            plain,
            "old signatures still verify"
        );

        release.runs_zed_extension = true;
        assert!(
            matches!(release.validate(), Err(Error::Invalid(message)) if message.contains("T2"))
        );
        release.tier = Tier::T2;
        release.validate().expect("T2 with the flag");
        let json = serde_json::to_value(&release).expect("json");
        assert_eq!(json["runs_zed_extension"], true);
        release.runs_zed_extension = false;
        let json = serde_json::to_value(&release).expect("json");
        assert!(
            json.get("runs_zed_extension").is_none(),
            "omitted when false"
        );
    }

    #[test]
    fn package_bytes_are_checked() {
        let release = release("todo", "1.0.0");
        assert_eq!(release.verify_package(b"package"), Ok(()));
        assert!(matches!(
            release.verify_package(b"packagf"),
            Err(Error::PackageMismatch(_))
        ));
        assert!(matches!(
            release.verify_package(b"package!"),
            Err(Error::PackageMismatch(_))
        ));
    }

    fn arbitrary_text(max: usize) -> impl Strategy<Value = String> {
        proptest::string::string_regex(&format!("[a-zA-Z0-9 .,:!?\u{e9}\u{4e2d}]{{0,{max}}}"))
            .expect("regex")
    }

    fn arbitrary_release() -> impl Strategy<Value = Release> {
        (
            (
                "[a-z][a-z0-9-]{0,20}[a-z0-9]",
                (0u64..100, 0u64..100, 0u64..100),
                arbitrary_text(40),
                arbitrary_text(80),
                proptest::collection::btree_set("[a-zA-Z0-9]{1,10}", 1..4),
                any::<[u8; 32]>(),
            ),
            (
                any::<u64>(),
                any::<u64>(),
                any::<bool>(),
                prop_oneof![Just(Tier::T0), Just(Tier::T1), Just(Tier::T2)],
                proptest::collection::btree_set("[a-z]{1,8}:[a-z]{1,8}", 0..4),
                proptest::collection::btree_set("[a-z]{1,8}\\.[a-z]{2,4}", 0..3),
                proptest::collection::btree_set("[a-z]{1,8}", 0..3),
                any::<bool>(),
            ),
        )
            .prop_map(
                |(
                    (id, (major, minor, patch), name, description, owners, sha256),
                    (
                        size,
                        published_at,
                        urgent,
                        tier,
                        capabilities,
                        network_domains,
                        processes,
                        runs_zed_extension,
                    ),
                )| {
                    let mut release = release(&id, "0.0.0");
                    release.version = Version::new(major, minor, patch);
                    release.name = name;
                    release.description = description;
                    release.owners = owners;
                    release.package.sha256 = Sha256Digest(sha256);
                    release.package.size = size;
                    release.published_at = Timestamp(published_at);
                    release.urgent = urgent;
                    release.tier = tier;
                    release.capabilities = capabilities;
                    release.network_domains = network_domains;
                    release.processes = processes;
                    release.runs_zed_extension = runs_zed_extension;
                    release
                },
            )
    }

    proptest! {
        #[test]
        fn canonical_bytes_decode_to_the_same_release(release in arbitrary_release()) {
            prop_assert_eq!(decode_release(&release.canonical_bytes()), Some(release));
        }

        #[test]
        fn json_round_trip_keeps_canonical_bytes(release in arbitrary_release()) {
            let json = serde_json::to_vec(&release).expect("serializes");
            let parsed: Release = serde_json::from_slice(&json).expect("parses");
            prop_assert_eq!(parsed.canonical_bytes(), release.canonical_bytes());
            let pretty = serde_json::to_vec_pretty(&release).expect("serializes");
            let parsed: Release = serde_json::from_slice(&pretty).expect("parses");
            prop_assert_eq!(parsed, release);
        }

        #[test]
        fn different_releases_have_different_canonical_bytes(
            left in arbitrary_release(),
            right in arbitrary_release(),
        ) {
            prop_assume!(left != right);
            prop_assert_ne!(left.canonical_bytes(), right.canonical_bytes());
        }

        #[test]
        fn signed_release_survives_json_and_verifies(release in arbitrary_release()) {
            let signed = signed(release);
            let json = serde_json::to_vec(&signed).expect("serializes");
            let parsed = Release::from_json(&json).expect("parses");
            prop_assert_eq!(parsed.verify(&keyset()), Ok(()));
        }

        // A changed byte anywhere in a signed release's JSON either breaks parsing, breaks the
        // signature, or means the same release (e.g. one whitespace character for another).
        #[test]
        fn flipped_json_byte_is_refused(
            release in arbitrary_release(),
            position in any::<prop::sample::Index>(),
            mask in 1u8..=255,
        ) {
            let signed = signed(release);
            let mut json = serde_json::to_vec_pretty(&signed).expect("serializes");
            let position = position.index(json.len());
            json[position] ^= mask;
            if let Ok(parsed) = Release::from_json(&json)
                && parsed.verify(&keyset()).is_ok()
            {
                prop_assert_eq!(parsed.signed.canonical_bytes(), signed.signed.canonical_bytes());
            }
        }

        #[test]
        fn flipped_canonical_byte_is_refused(
            release in arbitrary_release(),
            position in any::<prop::sample::Index>(),
            mask in 1u8..=255,
        ) {
            let keyset = keyset();
            let signed = signed(release);
            let mut bytes = signed.signed.canonical_bytes();
            let position = position.index(bytes.len());
            bytes[position] ^= mask;
            prop_assert!(keyset.verify(&bytes, &signed.signature, KeyRole::Release).is_err());
        }
    }
}
