//! Client-side core of the Fyx extension registry (Platform 1, decisions 14–21).
//!
//! No network, no file system, no clock. Callers pass in the bytes they (or a [`Fetch`] they
//! supply) fetched and the current time, and get back verified documents and decisions; the
//! [`RegistryClient`] ties them together in the order of trust. The formats are documented in
//! `docs/fyx/platform/registry-format.md`.

mod blocklist;
mod canonical;
mod client;
mod index;
mod package;
mod signing;
mod snapshot;
mod text;
mod update;

use std::{fmt, time::Duration};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use sha2::{Digest as _, Sha256};

pub use blocklist::{
    BLOCKLIST_FETCH_INTERVAL, BLOCKLIST_SCHEMA, BLOCKLIST_STALE_AFTER, BlockEntry, Blocklist,
    BlocklistCache, CacheUpdate, Comparator, ComparatorOp, Freshness, KeepReason,
    MAX_BLOCK_ENTRIES, MAX_BLOCKLIST_SIZE, MAX_BLOCKLIST_VALIDITY, VerifiedBlocklist, VersionRange,
};
pub use client::{
    DownloadError, Fetch, FetchRequest, IndexRefresh, InstallError, InstallPlan, RegistryClient,
    RegistryFile, StoredState, Unavailable,
};
pub use index::{
    INDEX_SCHEMA, Index, MAX_INDEX_SIZE, MAX_PACKAGE_SIZE, MAX_RELEASES, MalformedRelease, Package,
    RejectedRelease, Release, Source, VerifiedIndex,
};
pub use package::{
    RUNS_ZED_EXTENSION_WARNING, ZED_EXTENSION_WASM, entry_names, is_zed_extension_wasm,
    runs_zed_extension,
};
pub use signing::{KeyId, KeyRole, Keyset, PublicKey, Signature, SigningKey};
pub use snapshot::{MAX_SNAPSHOT_VALIDITY, SNAPSHOT_SCHEMA, Snapshot, VerifiedSnapshot};
pub use text::is_public_suffix;
pub use update::{
    AGE_GATE, Action, Additions, AgeGate, FirstSeen, HoldReason, InstallRefusal,
    InstalledExtension, InstalledVersion, Origin, Permissions, PlanChanges, PlannedAction,
    UpdateReason, age_gate, age_gate_seen, check_local_version, plan_install, plan_updates,
};

/// The documents this crate reads, for error messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Document {
    Index,
    Release,
    Blocklist,
    BlocklistCache,
    Keyset,
    SigningKey,
    Snapshot,
}

impl fmt::Display for Document {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Document::Index => "index.json",
            Document::Release => "release",
            Document::Blocklist => "blocklist.json",
            Document::BlocklistCache => "cached block list",
            Document::Keyset => "keyset",
            Document::SigningKey => "signing key",
            Document::Snapshot => "snapshot.json",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("{document} could not be read: {message}")]
    Parse { document: Document, message: String },
    #[error("{document} uses schema {found}; this version of Fyx reads schema {supported}")]
    UnsupportedSchema {
        document: Document,
        found: u64,
        supported: u32,
    },
    #[error("{0}")]
    Invalid(String),
    #[error("signed by unknown key {key_id:?}")]
    UnknownKey { key_id: String },
    #[error("key {key_id:?} may not sign a {role} document")]
    WrongRole { key_id: String, role: KeyRole },
    #[error("the signature by key {key_id:?} does not match the content")]
    BadSignature { key_id: String },
    #[error("package does not match the index: {0}")]
    PackageMismatch(String),
    /// A signed document past its `expires_at`: a mirror may be replaying an old copy.
    #[error("{document} expired at {expires_at:?}")]
    Expired {
        document: Document,
        expires_at: Timestamp,
    },
    /// A signed document older than one already seen: a mirror is replaying an old copy.
    #[error("{document} has sequence {found}, older than the {highest_seen} already seen")]
    Rollback {
        document: Document,
        found: u64,
        highest_seen: u64,
    },
}

/// Seconds since the Unix epoch, UTC. The crate never reads the clock; callers pass `now`.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Timestamp(pub u64);

impl Timestamp {
    pub fn saturating_add(self, duration: Duration) -> Self {
        Self(self.0.saturating_add(duration.as_secs()))
    }

    /// How long ago `earlier` was; zero if it is in the future.
    pub fn since(self, earlier: Timestamp) -> Duration {
        Duration::from_secs(self.0.saturating_sub(earlier.0))
    }
}

const MAX_EXTENSION_ID_LENGTH: usize = 64;

/// An extension id as in `extension.toml`: lowercase ASCII letters, digits and `-`, starting
/// with a letter and not ending with `-`, at most 64 characters.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExtensionId(String);

impl ExtensionId {
    pub fn new(value: impl Into<String>) -> Result<Self, Error> {
        let value = value.into();
        let valid = value.len() <= MAX_EXTENSION_ID_LENGTH
            && value.starts_with(|character: char| character.is_ascii_lowercase())
            && !value.ends_with('-')
            && value
                .chars()
                .all(|character| matches!(character, 'a'..='z' | '0'..='9' | '-'));
        if !valid {
            return Err(Error::Invalid(format!(
                "extension id {value:?} must be 1 to {MAX_EXTENSION_ID_LENGTH} characters of \
                 a-z, 0-9 and '-', starting with a letter and not ending with '-'"
            )));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ExtensionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for ExtensionId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ExtensionId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

/// Capability tiers by risk (decision 21). The order is the order of risk.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// No prompt; shown on the extension page.
    T0,
    /// One dialog at install.
    T1,
    /// Asked at first use, per extension, revocable.
    T2,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::T0 => "t0",
            Tier::T1 => "t1",
            Tier::T2 => "t2",
        }
    }

    /// Every capability this version of Fyx knows, with its tier. The registry template's
    /// `policy/capability-tiers.json` must list exactly these (a test checks it). The T3
    /// capabilities of fyx:ui 0.4 ([`Tier::T3_CAPABILITIES`]) count as T2 here: a release's tier
    /// has no T3 value (design wit-0.4 §C.1).
    pub const KNOWN_CAPABILITIES: [(&'static str, Tier); 35] = [
        ("ui:panel", Tier::T0),
        ("ui:status-bar", Tier::T0),
        ("ui:modal", Tier::T0),
        ("ui:notifications", Tier::T0),
        ("ui:editor-decorations", Tier::T0),
        ("ui:picker", Tier::T0),
        ("ui:code-lens", Tier::T0),
        ("ui:hover", Tier::T0),
        ("ui:virtual-documents", Tier::T0),
        ("ui:commands", Tier::T0),
        ("ui:images", Tier::T0),
        ("ui:canvas", Tier::T0),
        // fyx:ui 0.4.
        ("package:files", Tier::T0),
        ("ui:approvals", Tier::T0),
        ("workspace:read-buffers", Tier::T1),
        ("workspace:read-files", Tier::T1),
        ("workspace:diagnostics", Tier::T1),
        // fyx:ui 0.4.
        ("context:editor", Tier::T1),
        ("workspace:open", Tier::T1),
        ("terminal:profiles", Tier::T1),
        ("terminal:pseudo", Tier::T1),
        ("terminal:status", Tier::T1),
        ("review:propose", Tier::T1),
        ("network:fetch", Tier::T2),
        ("process:spawn", Tier::T2),
        ("workspace:edit", Tier::T2),
        ("agent:observe", Tier::T2),
        // fyx:ui 0.4.
        ("network:listen", Tier::T2),
        ("terminal:create", Tier::T2),
        ("terminal:env", Tier::T2),
        ("terminal:observe", Tier::T2),
        ("fs:discovery-file", Tier::T2),
        ("relay:hooks", Tier::T2),
        ("terminal:read", Tier::T2),
        ("terminal:write", Tier::T2),
    ];

    /// Asked at the moment of action, never in advance (fyx:ui 0.4, design §C.3): `t3` in the
    /// template's file, T2 for a release's tier.
    pub const T3_CAPABILITIES: [&'static str; 2] = ["terminal:read", "terminal:write"];

    /// The tier of a capability this version of Fyx knows, `None` for any other.
    pub fn of_capability(capability: &str) -> Option<Tier> {
        Self::KNOWN_CAPABILITIES
            .iter()
            .find(|(known, _)| *known == capability)
            .map(|(_, tier)| *tier)
    }

    /// Like [`Tier::of_capability`], but a capability this version does not know counts as T2:
    /// nothing unknown is ever granted silently.
    pub fn of_capability_or_highest(capability: &str) -> Tier {
        Self::of_capability(capability).unwrap_or(Tier::T2)
    }
}

impl fmt::Display for Tier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A SHA-256 digest, written as 64 lowercase hex digits.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Sha256Digest(pub [u8; 32]);

impl Sha256Digest {
    pub fn of(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }

    pub fn parse(text: &str) -> Result<Self, Error> {
        let invalid = || Error::Invalid(format!("{text:?} is not 64 lowercase hex digits"));
        if text.len() != 64
            || !text
                .chars()
                .all(|character| matches!(character, '0'..='9' | 'a'..='f'))
        {
            return Err(invalid());
        }
        let mut bytes = [0; 32];
        hex::decode_to_slice(text, &mut bytes).map_err(|_| invalid())?;
        Ok(Self(bytes))
    }
}

impl fmt::Display for Sha256Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&hex::encode(self.0))
    }
}

impl fmt::Debug for Sha256Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Sha256Digest({self})")
    }
}

impl Serialize for Sha256Digest {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Sha256Digest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(&String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

/// A document and the signature over its canonical bytes. In JSON:
/// `{"signed": {...}, "signature": {"key_id": "...", "ed25519": "<base64>"}}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Signed<T> {
    pub signed: T,
    pub signature: Signature,
}

/// Documents with a canonical byte form that signatures cover.
pub trait Canonical {
    /// The key role allowed to sign this document.
    const ROLE: KeyRole;

    fn canonical_bytes(&self) -> Vec<u8>;
}

impl<T: Canonical> Signed<T> {
    pub fn sign(signed: T, key: &SigningKey) -> Self {
        let signature = key.sign(&signed.canonical_bytes());
        Self { signed, signature }
    }

    pub fn verify(&self, keyset: &Keyset) -> Result<(), Error> {
        keyset.verify(&self.signed.canonical_bytes(), &self.signature, T::ROLE)
    }
}

/// Reads the schema number before the full parse, so a document from a newer registry gets
/// "unsupported schema" instead of a confusing field error. `pointer` is a JSON pointer to the
/// schema field.
fn check_schema(
    bytes: &[u8],
    document: Document,
    pointer: &str,
    supported: u32,
) -> Result<(), Error> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|error| Error::Parse {
        document,
        message: error.to_string(),
    })?;
    let found = value
        .pointer(pointer)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| Error::Parse {
            document,
            message: format!("missing schema number at {pointer}"),
        })?;
    if found != u64::from(supported) {
        return Err(Error::UnsupportedSchema {
            document,
            found,
            supported,
        });
    }
    Ok(())
}

fn parse_json<T: de::DeserializeOwned>(bytes: &[u8], document: Document) -> Result<T, Error> {
    serde_json::from_slice(bytes).map_err(|error| Error::Parse {
        document,
        message: error.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_ids_are_restricted() {
        for valid in ["a", "fyx-demo-counter", "a1", "x".repeat(64).as_str()] {
            assert!(ExtensionId::new(valid).is_ok(), "{valid}");
        }
        for invalid in [
            "",
            "1a",
            "-a",
            "a-",
            "A",
            "a_b",
            "a.b",
            "a/b",
            "x".repeat(65).as_str(),
        ] {
            assert!(ExtensionId::new(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn digests_are_lowercase_hex() {
        let digest = Sha256Digest::of(b"abc");
        assert_eq!(
            digest.to_string(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(Sha256Digest::parse(&digest.to_string()), Ok(digest));
        assert!(Sha256Digest::parse(&digest.to_string().to_uppercase()).is_err());
        assert!(Sha256Digest::parse("ab").is_err());
    }

    #[test]
    fn tiers_follow_decision_21() {
        assert_eq!(Tier::of_capability("ui:panel"), Some(Tier::T0));
        assert_eq!(Tier::of_capability("workspace:read-files"), Some(Tier::T1));
        assert_eq!(Tier::of_capability("network:fetch"), Some(Tier::T2));
        assert_eq!(Tier::of_capability("ui:web"), None);
        assert_eq!(Tier::of_capability_or_highest("ui:web"), Tier::T2);
        assert!(Tier::T0 < Tier::T1 && Tier::T1 < Tier::T2);
    }

    /// The registry template's policy script reads its tiers from this file; it must agree with
    /// the client, entry for entry.
    #[test]
    fn the_registry_template_knows_the_same_tiers() {
        let file: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tools/fyx-extensions-template/policy/capability-tiers.json"
        ))
        .expect("JSON");
        let listed: std::collections::BTreeMap<String, String> =
            serde_json::from_value(file["capabilities"].clone()).expect("capability -> tier");
        // The file writes T3 as `t3`; a release counts it as T2.
        let known: std::collections::BTreeMap<String, String> = Tier::KNOWN_CAPABILITIES
            .iter()
            .map(|(capability, tier)| {
                let tier = if Tier::T3_CAPABILITIES.contains(capability) {
                    assert_eq!(*tier, Tier::T2, "{capability}");
                    "t3"
                } else {
                    tier.as_str()
                };
                (capability.to_string(), tier.to_string())
            })
            .collect();
        assert_eq!(listed, known);
    }

    #[test]
    fn schema_check_reports_newer_documents() {
        assert_eq!(
            check_schema(br#"{"schema": 2}"#, Document::Index, "/schema", 1),
            Err(Error::UnsupportedSchema {
                document: Document::Index,
                found: 2,
                supported: 1
            })
        );
        assert!(check_schema(br#"{"schema": 1}"#, Document::Index, "/schema", 1).is_ok());
        assert!(check_schema(br#"{}"#, Document::Index, "/schema", 1).is_err());
    }
}
