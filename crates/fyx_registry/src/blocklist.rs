//! `blocklist.json` and the client's cached copy of it (decision 19, spec §4 failure 5).
//!
//! The block list is one signed document with a sequence number. The client keeps the last good
//! copy: a failed or invalid fetch never changes it, an older sequence never replaces a newer
//! one (otherwise replaying an old list would unblock versions), and being offline never
//! disables anything; it only makes the page warn once the copy is 7 days old.
//!
//! Its age is measured from the signed `generated_at`, and it carries a signed `expires_at`
//! (security-2 S5): a mirror that keeps serving the newest list it has, after the registry
//! published a newer one, cannot make it look fresh. The registry CI publishes a new list, with a
//! new sequence, at least daily.

use std::{fmt, time::Duration};

use semver::Version;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

use crate::{
    Canonical, Document, Error, ExtensionId, KeyRole, Keyset, Sha256Digest, Signed, Timestamp,
    canonical::CanonicalWriter, check_schema, index::to_pretty_json, parse_json, text::check_text,
};

pub const BLOCKLIST_SCHEMA: u32 = 1;
pub const BLOCKLIST_FETCH_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
pub const BLOCKLIST_STALE_AFTER: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// The longest a block list may be valid.
pub const MAX_BLOCKLIST_VALIDITY: Duration = BLOCKLIST_STALE_AFTER;
/// The most `blocklist.json` may weigh, and the most entries it may list.
pub const MAX_BLOCKLIST_SIZE: usize = 4 * 1024 * 1024;
pub const MAX_BLOCK_ENTRIES: usize = 10_000;

const BLOCKLIST_DOMAIN: &str = "fyx-registry/blocklist/v1";
const MAX_REASON_LENGTH: usize = 300;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Blocklist {
    pub schema: u32,
    /// Increases with every published list; a client never goes back to a lower one.
    pub sequence: u64,
    pub generated_at: Timestamp,
    /// After this the list is [`Freshness::Stale`], whatever the fetch time says.
    pub expires_at: Timestamp,
    pub entries: Vec<BlockEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockEntry {
    pub id: ExtensionId,
    pub versions: VersionRange,
    /// Blocks only the package with this hash (within `versions`), so a file from disk cannot
    /// escape a block by claiming another version (security-2 S13).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_sha256: Option<Sha256Digest>,
    /// Shown to the user in the notification and on the extension page.
    pub reason: String,
    pub urgent: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComparatorOp {
    Exact,
    Greater,
    GreaterOrEqual,
    Less,
    LessOrEqual,
}

impl ComparatorOp {
    fn symbol(self) -> &'static str {
        match self {
            ComparatorOp::Exact => "=",
            ComparatorOp::Greater => ">",
            ComparatorOp::GreaterOrEqual => ">=",
            ComparatorOp::Less => "<",
            ComparatorOp::LessOrEqual => "<=",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comparator {
    pub op: ComparatorOp,
    pub version: Version,
}

impl Comparator {
    /// Semver precedence, which ignores build metadata: `1.2.0+local` is `=1.2.0` (bugs-1 #21).
    pub fn matches(&self, version: &Version) -> bool {
        let ordering = version.cmp_precedence(&self.version);
        match self.op {
            ComparatorOp::Exact => ordering.is_eq(),
            ComparatorOp::Greater => ordering.is_gt(),
            ComparatorOp::GreaterOrEqual => ordering.is_ge(),
            ComparatorOp::Less => ordering.is_lt(),
            ComparatorOp::LessOrEqual => ordering.is_le(),
        }
    }
}

/// Versions a block entry covers: `*` for all, or comparators joined by `,` that must all hold,
/// e.g. `>=1.2.0, <1.4.0` or `=1.3.1`. Every comparator needs an explicit operator and a full
/// `major.minor.patch`. Unlike Cargo requirements there is no `^`/`~`, and pre-release versions
/// are compared by plain semver order, so `>=1.2.0` also covers `1.3.0-beta`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionRange {
    comparators: Vec<Comparator>,
}

impl VersionRange {
    pub fn all() -> Self {
        Self {
            comparators: Vec::new(),
        }
    }

    pub fn parse(text: &str) -> Result<Self, Error> {
        let text = text.trim();
        if text == "*" {
            return Ok(Self::all());
        }
        let comparators = text
            .split(',')
            .map(|part| {
                let part = part.trim();
                let invalid = || {
                    Error::Invalid(format!(
                        "version range part {part:?} must be an operator (=, >, >=, <, <=) \
                         followed by major.minor.patch"
                    ))
                };
                let (op, version) = [
                    (">=", ComparatorOp::GreaterOrEqual),
                    ("<=", ComparatorOp::LessOrEqual),
                    (">", ComparatorOp::Greater),
                    ("<", ComparatorOp::Less),
                    ("=", ComparatorOp::Exact),
                ]
                .into_iter()
                .find_map(|(symbol, op)| part.strip_prefix(symbol).map(|rest| (op, rest)))
                .ok_or_else(invalid)?;
                let version = Version::parse(version.trim()).map_err(|_| invalid())?;
                if !version.pre.is_empty() || !version.build.is_empty() {
                    return Err(invalid());
                }
                Ok(Comparator { op, version })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        Ok(Self { comparators })
    }

    pub fn comparators(&self) -> &[Comparator] {
        &self.comparators
    }

    pub fn matches(&self, version: &Version) -> bool {
        self.comparators
            .iter()
            .all(|comparator| comparator.matches(version))
    }
}

impl fmt::Display for VersionRange {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.comparators.is_empty() {
            return formatter.write_str("*");
        }
        for (position, comparator) in self.comparators.iter().enumerate() {
            if position > 0 {
                formatter.write_str(", ")?;
            }
            write!(
                formatter,
                "{}{}",
                comparator.op.symbol(),
                comparator.version
            )?;
        }
        Ok(())
    }
}

impl Serialize for VersionRange {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for VersionRange {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(&String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

impl Canonical for Blocklist {
    const ROLE: KeyRole = KeyRole::Blocklist;

    fn canonical_bytes(&self) -> Vec<u8> {
        let mut writer = CanonicalWriter::new(BLOCKLIST_DOMAIN);
        writer.u64(u64::from(self.schema));
        writer.u64(self.sequence);
        writer.u64(self.generated_at.0);
        writer.u64(self.expires_at.0);
        writer.length(self.entries.len());
        for entry in &self.entries {
            writer.string(entry.id.as_str());
            writer.length(entry.versions.comparators.len());
            for comparator in &entry.versions.comparators {
                writer.string(comparator.op.symbol());
                writer.string(&comparator.version.to_string());
            }
            writer.string(&entry.reason);
            writer.bool(entry.urgent);
            writer.bool(entry.package_sha256.is_some());
            if let Some(sha256) = &entry.package_sha256 {
                writer.byte_string(&sha256.0);
            }
        }
        writer.finish()
    }
}

impl Blocklist {
    pub fn validate(&self) -> Result<(), Error> {
        if self.schema != BLOCKLIST_SCHEMA {
            return Err(Error::UnsupportedSchema {
                document: Document::Blocklist,
                found: u64::from(self.schema),
                supported: BLOCKLIST_SCHEMA,
            });
        }
        if self.expires_at <= self.generated_at
            || self.expires_at.0 - self.generated_at.0 > MAX_BLOCKLIST_VALIDITY.as_secs()
        {
            return Err(Error::Invalid(format!(
                "block list {}: expires_at must be after generated_at and at most {} days later",
                self.sequence,
                MAX_BLOCKLIST_VALIDITY.as_secs() / 86_400
            )));
        }
        if self.entries.len() > MAX_BLOCK_ENTRIES {
            return Err(Error::Invalid(format!(
                "block list {}: at most {MAX_BLOCK_ENTRIES} entries",
                self.sequence
            )));
        }
        for entry in &self.entries {
            check_text("reason", &entry.reason, 1, MAX_REASON_LENGTH).map_err(|message| {
                Error::Invalid(format!(
                    "block entry for {} {}: {message}",
                    entry.id, entry.versions
                ))
            })?;
        }
        Ok(())
    }

    /// Parses a signed `blocklist.json` without verifying it.
    pub fn from_json(bytes: &[u8]) -> Result<Signed<Blocklist>, Error> {
        if bytes.len() > MAX_BLOCKLIST_SIZE {
            return Err(Error::Invalid(format!(
                "blocklist.json has {} bytes, the cap is {MAX_BLOCKLIST_SIZE}",
                bytes.len()
            )));
        }
        check_schema(
            bytes,
            Document::Blocklist,
            "/signed/schema",
            BLOCKLIST_SCHEMA,
        )?;
        parse_json(bytes, Document::Blocklist)
    }
}

/// A block list whose signature and contents check out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedBlocklist {
    signed: Signed<Blocklist>,
}

impl VerifiedBlocklist {
    pub fn verify(signed: Signed<Blocklist>, keyset: &Keyset) -> Result<Self, Error> {
        signed.verify(keyset)?;
        signed.signed.validate()?;
        Ok(Self { signed })
    }

    pub fn from_json(bytes: &[u8], keyset: &Keyset) -> Result<Self, Error> {
        Self::verify(Blocklist::from_json(bytes)?, keyset)
    }

    pub fn signed(&self) -> &Signed<Blocklist> {
        &self.signed
    }

    pub fn sequence(&self) -> u64 {
        self.signed.signed.sequence
    }

    pub fn entries(&self) -> &[BlockEntry] {
        &self.signed.signed.entries
    }

    /// The first entry that blocks `version` of `id` whose package has `package_sha256` (if
    /// known). An entry with a hash only blocks a package known to have it.
    pub fn block_for(
        &self,
        id: &ExtensionId,
        version: &Version,
        package_sha256: Option<&Sha256Digest>,
    ) -> Option<&BlockEntry> {
        self.entries().iter().find(|entry| {
            &entry.id == id
                && entry.versions.matches(version)
                && entry
                    .package_sha256
                    .as_ref()
                    .is_none_or(|blocked| package_sha256 == Some(blocked))
        })
    }

    pub fn is_blocked(
        &self,
        id: &ExtensionId,
        version: &Version,
        package_sha256: Option<&Sha256Digest>,
    ) -> bool {
        self.block_for(id, version, package_sha256).is_some()
    }

    pub fn generated_at(&self) -> Timestamp {
        self.signed.signed.generated_at
    }

    pub fn expires_at(&self) -> Timestamp {
        self.signed.signed.expires_at
    }
}

/// The last good block list and when it was fetched, as stored on disk:
/// `{"fetched_at": <unix seconds>, "blocklist": <signed blocklist.json>}`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlocklistCache {
    pub fetched_at: Timestamp,
    pub blocklist: VerifiedBlocklist,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BlocklistCacheFile {
    fetched_at: Timestamp,
    blocklist: Signed<Blocklist>,
}

/// What to do with the cache after a fetch attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CacheUpdate {
    /// A newer list: store it and apply it.
    Replace(BlocklistCache),
    /// The same list again: store the new `fetched_at`, nothing else changes.
    Refresh(BlocklistCache),
    /// Keep the cache as it is; the reason is for the log.
    Keep(KeepReason),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeepReason {
    FetchFailed(String),
    Invalid(Error),
    OlderSequence {
        cached: u64,
        fetched: u64,
    },
    /// The registry published two different lists with one sequence number.
    ConflictingSequence {
        sequence: u64,
    },
}

/// How current the cached block list is, for the extensions page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Freshness {
    /// Never fetched successfully. Nothing is blocked until a fetch succeeds.
    Missing,
    Fresh,
    /// Show "Block list is N days old". Nothing is disabled for being offline.
    Stale {
        age: Duration,
    },
}

impl BlocklistCache {
    pub fn from_json(bytes: &[u8], keyset: &Keyset) -> Result<Self, Error> {
        check_schema(
            bytes,
            Document::BlocklistCache,
            "/blocklist/signed/schema",
            BLOCKLIST_SCHEMA,
        )?;
        let file: BlocklistCacheFile = parse_json(bytes, Document::BlocklistCache)?;
        Ok(Self {
            fetched_at: file.fetched_at,
            blocklist: VerifiedBlocklist::verify(file.blocklist, keyset)?,
        })
    }

    pub fn to_json(&self) -> Result<Vec<u8>, Error> {
        to_pretty_json(
            &BlocklistCacheFile {
                fetched_at: self.fetched_at,
                blocklist: self.blocklist.signed.clone(),
            },
            Document::BlocklistCache,
        )
    }

    /// Decides what a fetch attempt does to the cache. `fetched` is the response body, or the
    /// error the fetch failed with.
    pub fn apply_fetch(
        cache: Option<&BlocklistCache>,
        fetched: Result<&[u8], String>,
        keyset: &Keyset,
        now: Timestamp,
    ) -> CacheUpdate {
        let body = match fetched {
            Ok(body) => body,
            Err(error) => return CacheUpdate::Keep(KeepReason::FetchFailed(error)),
        };
        let blocklist = match VerifiedBlocklist::from_json(body, keyset) {
            Ok(blocklist) => blocklist,
            Err(error) => return CacheUpdate::Keep(KeepReason::Invalid(error)),
        };
        let fresh = BlocklistCache {
            fetched_at: now,
            blocklist,
        };
        let Some(cache) = cache else {
            return CacheUpdate::Replace(fresh);
        };
        let cached = cache.blocklist.sequence();
        let fetched = fresh.blocklist.sequence();
        if fetched > cached {
            CacheUpdate::Replace(fresh)
        } else if fetched < cached {
            CacheUpdate::Keep(KeepReason::OlderSequence { cached, fetched })
        } else if fresh.blocklist.signed.signed.canonical_bytes()
            == cache.blocklist.signed.signed.canonical_bytes()
        {
            CacheUpdate::Refresh(BlocklistCache {
                // A clock that went backwards must not make the cache look older.
                fetched_at: now.max(cache.fetched_at),
                blocklist: cache.blocklist.clone(),
            })
        } else {
            CacheUpdate::Keep(KeepReason::ConflictingSequence { sequence: cached })
        }
    }

    /// The age counts from the list's signed `generated_at` (a list stamped in the future is
    /// age 0), not from when it was fetched, and a list past its signed `expires_at` is stale
    /// however recently it was fetched.
    pub fn freshness(cache: Option<&BlocklistCache>, now: Timestamp) -> Freshness {
        let Some(cache) = cache else {
            return Freshness::Missing;
        };
        let age = now.since(cache.blocklist.generated_at());
        if age >= BLOCKLIST_STALE_AFTER || now >= cache.blocklist.expires_at() {
            Freshness::Stale { age }
        } else {
            Freshness::Fresh
        }
    }

    /// Whether to fetch now, given when the last attempt (successful or not) started. An attempt
    /// stamped in the future (the clock was fast) does not postpone the next one (bugs-1 #22).
    pub fn fetch_due(last_attempt: Option<Timestamp>, now: Timestamp) -> bool {
        last_attempt.is_none_or(|last_attempt| {
            last_attempt > now || now.since(last_attempt) >= BLOCKLIST_FETCH_INTERVAL
        })
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::{SigningKey, canonical::CanonicalReader};

    pub(crate) fn entry(id: &str, versions: &str) -> BlockEntry {
        BlockEntry {
            id: ExtensionId::new(id).expect("valid id"),
            versions: VersionRange::parse(versions).expect("valid range"),
            reason: "Uploads the open buffer to a third party.".into(),
            urgent: true,
            package_sha256: None,
        }
    }

    pub(crate) fn blocklist(sequence: u64, entries: Vec<BlockEntry>) -> Blocklist {
        Blocklist {
            schema: BLOCKLIST_SCHEMA,
            sequence,
            generated_at: Timestamp(sequence * 10),
            expires_at: Timestamp(sequence * 10 + 2 * 86_400),
            entries,
        }
    }

    pub(crate) fn signed_json(blocklist: Blocklist, key: &SigningKey) -> Vec<u8> {
        serde_json::to_vec(&Signed::sign(blocklist, key)).expect("serializes")
    }

    pub(crate) fn verified(blocklist: Blocklist) -> VerifiedBlocklist {
        VerifiedBlocklist::verify(
            Signed::sign(blocklist, &crate::signing::test_keys::registry_key()),
            &crate::signing::test_keys::keyset(),
        )
        .expect("verifies")
    }

    pub(crate) fn decode_blocklist(bytes: &[u8]) -> Option<Blocklist> {
        let mut reader = CanonicalReader::new(bytes, BLOCKLIST_DOMAIN)?;
        let schema = u32::try_from(reader.u64()?).ok()?;
        let sequence = reader.u64()?;
        let generated_at = Timestamp(reader.u64()?);
        let expires_at = Timestamp(reader.u64()?);
        let entry_count = reader.length()?;
        let mut entries = Vec::new();
        for _ in 0..entry_count {
            let id = ExtensionId::new(reader.string()?).ok()?;
            let comparator_count = reader.length()?;
            let mut comparators = Vec::new();
            for _ in 0..comparator_count {
                let op = match reader.string()?.as_str() {
                    "=" => ComparatorOp::Exact,
                    ">" => ComparatorOp::Greater,
                    ">=" => ComparatorOp::GreaterOrEqual,
                    "<" => ComparatorOp::Less,
                    "<=" => ComparatorOp::LessOrEqual,
                    _ => return None,
                };
                let version = Version::parse(&reader.string()?).ok()?;
                comparators.push(Comparator { op, version });
            }
            let reason = reader.string()?;
            let urgent = reader.bool()?;
            let package_sha256 = if reader.bool()? {
                Some(Sha256Digest(reader.byte_string()?.try_into().ok()?))
            } else {
                None
            };
            entries.push(BlockEntry {
                id,
                versions: VersionRange { comparators },
                reason,
                urgent,
                package_sha256,
            });
        }
        reader.finish()?;
        Some(Blocklist {
            schema,
            sequence,
            generated_at,
            expires_at,
            entries,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{test_support::*, *};
    use crate::signing::test_keys::{keyset, registry_key, signing_key};
    use proptest::prelude::*;

    fn version(text: &str) -> Version {
        Version::parse(text).expect("valid version")
    }

    #[test]
    fn ranges_parse_display_and_match() {
        let range = VersionRange::parse(" >=1.2.0 ,<1.4.0").expect("valid");
        assert_eq!(range.to_string(), ">=1.2.0, <1.4.0");
        assert!(!range.matches(&version("1.1.9")));
        assert!(range.matches(&version("1.2.0")));
        assert!(range.matches(&version("1.3.0-beta.1")));
        assert!(!range.matches(&version("1.4.0")));

        assert!(
            VersionRange::parse("*")
                .expect("valid")
                .matches(&version("9.9.9"))
        );
        let exact = VersionRange::parse("=1.3.1").expect("valid");
        assert!(exact.matches(&version("1.3.1")) && !exact.matches(&version("1.3.2")));
        let bounded = VersionRange::parse("> 1.0.0, <= 2.0.0").expect("valid");
        assert!(!bounded.matches(&version("1.0.0")) && bounded.matches(&version("2.0.0")));

        for invalid in [
            "",
            "1.2.0",
            "^1.2.0",
            "~1.2",
            ">=1.2",
            ">=1.2.0,",
            "=1.0.0-rc.1",
            "*, <1.0.0",
        ] {
            assert!(VersionRange::parse(invalid).is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn signed_blocklist_round_trips_and_verifies() {
        let list = blocklist(3, vec![entry("todo", ">=1.2.0, <1.3.0"), entry("spy", "*")]);
        let json = signed_json(list.clone(), &registry_key());
        let verified = VerifiedBlocklist::from_json(&json, &keyset()).expect("verifies");
        assert_eq!(verified.signed().signed, list);
        let todo = ExtensionId::new("todo").expect("valid");
        assert!(verified.is_blocked(&todo, &version("1.2.5"), None));
        assert!(!verified.is_blocked(&todo, &version("1.3.0"), None));
        assert!(verified.is_blocked(
            &ExtensionId::new("spy").expect("valid"),
            &version("0.0.1"),
            None
        ));
    }

    #[test]
    fn refuses_tampered_unknown_key_newer_schema_and_bad_reason() {
        let list = blocklist(3, vec![entry("todo", "=1.2.0")]);
        let mut tampered = Signed::sign(list.clone(), &registry_key());
        tampered.signed.entries[0].versions = VersionRange::parse("=1.2.1").expect("valid");
        assert!(matches!(
            VerifiedBlocklist::verify(tampered, &keyset()),
            Err(Error::BadSignature { .. })
        ));

        let json = signed_json(list.clone(), &signing_key("stranger", 9));
        assert!(matches!(
            VerifiedBlocklist::from_json(&json, &keyset()),
            Err(Error::UnknownKey { .. })
        ));

        let newer = br#"{"signed": {"schema": 2}, "signature": {}}"#;
        assert!(matches!(
            VerifiedBlocklist::from_json(newer, &keyset()),
            Err(Error::UnsupportedSchema { found: 2, .. })
        ));

        let mut empty_reason = list;
        empty_reason.entries[0].reason.clear();
        let json = signed_json(empty_reason, &registry_key());
        assert!(matches!(
            VerifiedBlocklist::from_json(&json, &keyset()),
            Err(Error::Invalid(_))
        ));
    }

    #[test]
    fn fetch_failures_and_invalid_lists_keep_the_cache() {
        let keyset = keyset();
        let cache = BlocklistCache {
            fetched_at: Timestamp(100),
            blocklist: verified(blocklist(5, vec![entry("todo", "=1.2.0")])),
        };

        assert_eq!(
            BlocklistCache::apply_fetch(
                Some(&cache),
                Err("timed out".into()),
                &keyset,
                Timestamp(200)
            ),
            CacheUpdate::Keep(KeepReason::FetchFailed("timed out".into()))
        );
        assert!(matches!(
            BlocklistCache::apply_fetch(Some(&cache), Ok(b"<html>"), &keyset, Timestamp(200)),
            CacheUpdate::Keep(KeepReason::Invalid(Error::Parse { .. }))
        ));
        let unsigned = signed_json(blocklist(6, Vec::new()), &signing_key("stranger", 9));
        assert!(matches!(
            BlocklistCache::apply_fetch(Some(&cache), Ok(&unsigned), &keyset, Timestamp(200)),
            CacheUpdate::Keep(KeepReason::Invalid(Error::UnknownKey { .. }))
        ));
        // An empty old list replayed by an attacker must not unblock anything.
        let replayed = signed_json(blocklist(4, Vec::new()), &registry_key());
        assert_eq!(
            BlocklistCache::apply_fetch(Some(&cache), Ok(&replayed), &keyset, Timestamp(200)),
            CacheUpdate::Keep(KeepReason::OlderSequence {
                cached: 5,
                fetched: 4
            })
        );
        let conflicting = signed_json(blocklist(5, Vec::new()), &registry_key());
        assert_eq!(
            BlocklistCache::apply_fetch(Some(&cache), Ok(&conflicting), &keyset, Timestamp(200)),
            CacheUpdate::Keep(KeepReason::ConflictingSequence { sequence: 5 })
        );
    }

    #[test]
    fn newer_lists_replace_and_the_same_list_refreshes() {
        let keyset = keyset();
        let current = blocklist(5, vec![entry("todo", "=1.2.0")]);
        let cache = BlocklistCache {
            fetched_at: Timestamp(100),
            blocklist: verified(current.clone()),
        };

        let same = signed_json(current, &registry_key());
        assert_eq!(
            BlocklistCache::apply_fetch(Some(&cache), Ok(&same), &keyset, Timestamp(200)),
            CacheUpdate::Refresh(BlocklistCache {
                fetched_at: Timestamp(200),
                blocklist: cache.blocklist.clone(),
            })
        );
        assert!(matches!(
            BlocklistCache::apply_fetch(Some(&cache), Ok(&same), &keyset, Timestamp(50)),
            CacheUpdate::Refresh(BlocklistCache {
                fetched_at: Timestamp(100),
                ..
            })
        ));

        let newer = blocklist(6, Vec::new());
        let json = signed_json(newer.clone(), &registry_key());
        assert_eq!(
            BlocklistCache::apply_fetch(Some(&cache), Ok(&json), &keyset, Timestamp(200)),
            CacheUpdate::Replace(BlocklistCache {
                fetched_at: Timestamp(200),
                blocklist: verified(newer),
            })
        );
        assert!(matches!(
            BlocklistCache::apply_fetch(None, Ok(&json), &keyset, Timestamp(200)),
            CacheUpdate::Replace(_)
        ));
    }

    #[test]
    fn cache_file_round_trips_and_is_verified_on_load() {
        let keyset = keyset();
        let cache = BlocklistCache {
            fetched_at: Timestamp(100),
            blocklist: verified(blocklist(5, vec![entry("todo", "=1.2.0")])),
        };
        let json = cache.to_json().expect("serializes");
        assert_eq!(BlocklistCache::from_json(&json, &keyset), Ok(cache));

        let tampered = String::from_utf8(json)
            .expect("utf-8")
            .replace("=1.2.0", "=1.2.1");
        assert!(matches!(
            BlocklistCache::from_json(tampered.as_bytes(), &keyset),
            Err(Error::BadSignature { .. })
        ));
    }

    #[test]
    fn freshness_counts_from_the_signed_time_and_fetch_runs_every_six_hours() {
        let day = 24 * 60 * 60;
        let mut list = blocklist(1, Vec::new());
        list.generated_at = Timestamp(10 * day);
        list.expires_at = Timestamp(17 * day);
        let cache = BlocklistCache {
            fetched_at: Timestamp(10 * day),
            blocklist: verified(list.clone()),
        };
        assert_eq!(
            BlocklistCache::freshness(None, Timestamp(0)),
            Freshness::Missing
        );
        assert_eq!(
            BlocklistCache::freshness(Some(&cache), Timestamp(17 * day - 1)),
            Freshness::Fresh
        );
        assert_eq!(
            BlocklistCache::freshness(Some(&cache), Timestamp(17 * day)),
            Freshness::Stale {
                age: Duration::from_secs(7 * day)
            }
        );
        assert_eq!(
            BlocklistCache::freshness(Some(&cache), Timestamp(0)),
            Freshness::Fresh
        );

        // security-2 S5: the same list replayed every day refreshes `fetched_at`, but it still
        // goes stale when its signed expiry passes.
        list.expires_at = Timestamp(12 * day);
        let mut cache = BlocklistCache {
            fetched_at: Timestamp(10 * day),
            blocklist: verified(list.clone()),
        };
        let replayed = signed_json(list, &registry_key());
        for now in (11..20).map(|days| Timestamp(days * day)) {
            if let CacheUpdate::Refresh(refreshed) =
                BlocklistCache::apply_fetch(Some(&cache), Ok(&replayed), &keyset(), now)
            {
                cache = refreshed;
            }
        }
        assert_eq!(cache.fetched_at, Timestamp(19 * day));
        assert!(matches!(
            BlocklistCache::freshness(Some(&cache), Timestamp(19 * day)),
            Freshness::Stale { .. }
        ));

        let hour = 60 * 60;
        assert!(BlocklistCache::fetch_due(None, Timestamp(0)));
        assert!(!BlocklistCache::fetch_due(
            Some(Timestamp(0)),
            Timestamp(6 * hour - 1)
        ));
        assert!(BlocklistCache::fetch_due(
            Some(Timestamp(0)),
            Timestamp(6 * hour)
        ));
        // bugs-1 #22: a last attempt stamped a year ahead does not stop fetching.
        assert!(BlocklistCache::fetch_due(
            Some(Timestamp(365 * day)),
            Timestamp(day)
        ));
    }

    #[test]
    fn validity_and_size_are_bounded() {
        let mut list = blocklist(1, Vec::new());
        list.expires_at = list.generated_at;
        assert!(list.validate().is_err());
        list.expires_at = list.generated_at.saturating_add(MAX_BLOCKLIST_VALIDITY);
        assert!(list.validate().is_ok());
        list.expires_at = list.expires_at.saturating_add(Duration::from_secs(1));
        assert!(list.validate().is_err());

        let mut hidden = blocklist(1, vec![entry("todo", "*")]);
        hidden.entries[0].reason = "Spyware\u{202e}".into();
        assert!(hidden.validate().is_err());
        assert!(Blocklist::from_json(&vec![b' '; MAX_BLOCKLIST_SIZE + 1]).is_err());
    }

    /// bugs-1 #21, security-2 S13: build metadata does not escape a block, and hash entries
    /// block exactly one package.
    #[test]
    fn build_metadata_and_hash_entries() {
        let exact = VersionRange::parse("=1.2.0").expect("valid");
        assert!(exact.matches(&version("1.2.0+local")));
        assert!(
            VersionRange::parse("<=1.2.0")
                .expect("valid")
                .matches(&version("1.2.0+x"))
        );
        assert!(
            !VersionRange::parse(">1.2.0")
                .expect("valid")
                .matches(&version("1.2.0+x"))
        );

        let bad = Sha256Digest::of(b"bad package");
        let mut hashed = entry("todo", "*");
        hashed.package_sha256 = Some(bad);
        let list = verified(blocklist(2, vec![hashed]));
        let todo = ExtensionId::new("todo").expect("valid");
        assert!(list.is_blocked(&todo, &version("9.0.0"), Some(&bad)));
        assert!(!list.is_blocked(&todo, &version("9.0.0"), Some(&Sha256Digest::of(b"good"))));
        assert!(!list.is_blocked(&todo, &version("9.0.0"), None));
    }

    fn arbitrary_range() -> impl Strategy<Value = VersionRange> {
        let comparator = (
            prop_oneof![
                Just(ComparatorOp::Exact),
                Just(ComparatorOp::Greater),
                Just(ComparatorOp::GreaterOrEqual),
                Just(ComparatorOp::Less),
                Just(ComparatorOp::LessOrEqual),
            ],
            (0u64..20, 0u64..20, 0u64..20),
        )
            .prop_map(|(op, (major, minor, patch))| Comparator {
                op,
                version: Version::new(major, minor, patch),
            });
        proptest::collection::vec(comparator, 0..3)
            .prop_map(|comparators| VersionRange { comparators })
    }

    fn arbitrary_blocklist() -> impl Strategy<Value = Blocklist> {
        let entry = (
            "[a-z][a-z0-9-]{0,12}[a-z0-9]",
            arbitrary_range(),
            "[a-zA-Z0-9 .,]{1,40}",
            any::<bool>(),
            proptest::option::of(any::<[u8; 32]>()),
        )
            .prop_map(|(id, versions, reason, urgent, sha256)| BlockEntry {
                id: ExtensionId::new(id).expect("valid id"),
                versions,
                reason,
                urgent,
                package_sha256: sha256.map(Sha256Digest),
            });
        (
            any::<u64>(),
            0u64..1 << 40,
            1u64..=MAX_BLOCKLIST_VALIDITY.as_secs(),
            proptest::collection::vec(entry, 0..4),
        )
            .prop_map(|(sequence, generated_at, validity, entries)| Blocklist {
                schema: BLOCKLIST_SCHEMA,
                sequence,
                generated_at: Timestamp(generated_at),
                expires_at: Timestamp(generated_at + validity),
                entries,
            })
    }

    proptest! {
        #[test]
        fn range_text_round_trips(range in arbitrary_range()) {
            prop_assert_eq!(VersionRange::parse(&range.to_string()).expect("parses"), range);
        }

        #[test]
        fn canonical_bytes_decode_to_the_same_blocklist(list in arbitrary_blocklist()) {
            prop_assert_eq!(decode_blocklist(&list.canonical_bytes()), Some(list));
        }

        #[test]
        fn json_round_trip_keeps_canonical_bytes(list in arbitrary_blocklist()) {
            let json = signed_json(list.clone(), &registry_key());
            let parsed = Blocklist::from_json(&json).expect("parses");
            prop_assert_eq!(parsed.signed.canonical_bytes(), list.canonical_bytes());
            prop_assert_eq!(parsed.verify(&keyset()), Ok(()));
        }

        #[test]
        fn flipped_json_byte_is_refused(
            list in arbitrary_blocklist(),
            position in any::<prop::sample::Index>(),
            mask in 1u8..=255,
        ) {
            let mut json = serde_json::to_vec_pretty(&Signed::sign(list.clone(), &registry_key()))
                .expect("serializes");
            let position = position.index(json.len());
            json[position] ^= mask;
            if let Ok(verified) = VerifiedBlocklist::from_json(&json, &keyset()) {
                prop_assert_eq!(verified.signed().signed.canonical_bytes(), list.canonical_bytes());
            }
        }

        #[test]
        fn flipped_canonical_byte_is_refused(
            list in arbitrary_blocklist(),
            position in any::<prop::sample::Index>(),
            mask in 1u8..=255,
        ) {
            let signed = Signed::sign(list, &registry_key());
            let mut bytes = signed.signed.canonical_bytes();
            let position = position.index(bytes.len());
            bytes[position] ^= mask;
            prop_assert!(keyset().verify(&bytes, &signed.signature, KeyRole::Blocklist).is_err());
        }
    }
}
