//! `snapshot.json`: the signed statement of what the index is right now (security-2 S6, T9).
//!
//! Releases are signed one by one, so on its own `index.json` says nothing about which releases
//! exist or how current it is: a mirror could drop a security fix, or keep serving an old index,
//! and every release would still verify. The snapshot closes that: it names the exact index by
//! size and SHA-256, carries the signed time, expires (the registry CI re-signs it at least
//! daily), and has a sequence number that never goes down. It also lists revoked keys, whose
//! signatures nothing accepts any more.
//!
//! A client keeps the highest snapshot sequence it has seen and passes it to
//! [`VerifiedSnapshot::verify`]; like the block list, an older one never replaces a newer one.

use std::{collections::BTreeSet, time::Duration};

use serde::{Deserialize, Serialize};

use crate::{
    Canonical, Document, Error, KeyId, KeyRole, Keyset, MAX_INDEX_SIZE, Sha256Digest, Signed,
    Timestamp, canonical::CanonicalWriter, check_schema, index::to_pretty_json, parse_json,
};

pub const SNAPSHOT_SCHEMA: u32 = 1;
/// The longest a snapshot may be valid; the CI re-signs well before (spec: daily).
pub const MAX_SNAPSHOT_VALIDITY: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// The most `snapshot.json` may weigh.
pub const MAX_SNAPSHOT_SIZE: usize = 64 * 1024;
/// The most keys one snapshot may revoke.
pub const MAX_REVOKED_KEYS: usize = 256;

const SNAPSHOT_DOMAIN: &str = "fyx-registry/snapshot/v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub schema: u32,
    /// Increases with every snapshot, re-signing included.
    pub sequence: u64,
    pub generated_at: Timestamp,
    pub expires_at: Timestamp,
    /// The exact `index.json` this snapshot vouches for.
    pub index_sha256: Sha256Digest,
    pub index_size: u64,
    /// Keys whose signatures are refused on releases and block lists from now on.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub revoked_keys: BTreeSet<KeyId>,
}

impl Canonical for Snapshot {
    const ROLE: KeyRole = KeyRole::Snapshot;

    fn canonical_bytes(&self) -> Vec<u8> {
        let mut writer = CanonicalWriter::new(SNAPSHOT_DOMAIN);
        writer.u64(u64::from(self.schema));
        writer.u64(self.sequence);
        writer.u64(self.generated_at.0);
        writer.u64(self.expires_at.0);
        writer.byte_string(&self.index_sha256.0);
        writer.u64(self.index_size);
        writer.strings(self.revoked_keys.iter().map(KeyId::as_str));
        writer.finish()
    }
}

impl Snapshot {
    /// The snapshot for `index_bytes`, valid for `validity` from `generated_at`.
    pub fn for_index(
        index_bytes: &[u8],
        sequence: u64,
        generated_at: Timestamp,
        validity: Duration,
        revoked_keys: BTreeSet<KeyId>,
    ) -> Self {
        Self {
            schema: SNAPSHOT_SCHEMA,
            sequence,
            generated_at,
            expires_at: generated_at.saturating_add(validity),
            index_sha256: Sha256Digest::of(index_bytes),
            index_size: index_bytes.len() as u64,
            revoked_keys,
        }
    }

    pub fn validate(&self) -> Result<(), Error> {
        if self.schema != SNAPSHOT_SCHEMA {
            return Err(Error::UnsupportedSchema {
                document: Document::Snapshot,
                found: u64::from(self.schema),
                supported: SNAPSHOT_SCHEMA,
            });
        }
        if self.expires_at <= self.generated_at
            || self.expires_at.0 - self.generated_at.0 > MAX_SNAPSHOT_VALIDITY.as_secs()
        {
            return Err(Error::Invalid(format!(
                "snapshot {}: expires_at must be after generated_at and at most {} days later",
                self.sequence,
                MAX_SNAPSHOT_VALIDITY.as_secs() / 86_400
            )));
        }
        if self.index_size == 0 || self.index_size > MAX_INDEX_SIZE {
            return Err(Error::Invalid(format!(
                "snapshot {}: index_size {} must be between 1 and {MAX_INDEX_SIZE} bytes",
                self.sequence, self.index_size
            )));
        }
        if self.revoked_keys.len() > MAX_REVOKED_KEYS {
            return Err(Error::Invalid(format!(
                "snapshot {}: at most {MAX_REVOKED_KEYS} revoked keys",
                self.sequence
            )));
        }
        Ok(())
    }

    /// Parses a signed `snapshot.json` without verifying it.
    pub fn from_json(bytes: &[u8]) -> Result<Signed<Snapshot>, Error> {
        if bytes.len() > MAX_SNAPSHOT_SIZE {
            return Err(Error::Invalid(format!(
                "snapshot.json has {} bytes, the cap is {MAX_SNAPSHOT_SIZE}",
                bytes.len()
            )));
        }
        check_schema(bytes, Document::Snapshot, "/signed/schema", SNAPSHOT_SCHEMA)?;
        parse_json(bytes, Document::Snapshot)
    }

    pub fn to_json(signed: &Signed<Snapshot>) -> Result<Vec<u8>, Error> {
        to_pretty_json(signed, Document::Snapshot)
    }
}

/// A snapshot whose signature, contents, expiry and sequence check out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedSnapshot {
    signed: Signed<Snapshot>,
}

impl VerifiedSnapshot {
    /// `keyset` is the client's trusted keyset, minus the keys an earlier snapshot revoked
    /// ([`VerifiedSnapshot::effective_keyset`] of the last accepted one). `highest_seen` is the
    /// highest sequence accepted before; the same sequence is accepted again only with the same
    /// contents.
    pub fn verify(
        signed: Signed<Snapshot>,
        keyset: &Keyset,
        now: Timestamp,
        highest_seen: Option<(u64, &Signed<Snapshot>)>,
    ) -> Result<Self, Error> {
        signed.verify(keyset)?;
        signed.signed.validate()?;
        if now >= signed.signed.expires_at {
            return Err(Error::Expired {
                document: Document::Snapshot,
                expires_at: signed.signed.expires_at,
            });
        }
        if let Some((sequence, seen)) = highest_seen {
            let found = signed.signed.sequence;
            let replayed = found < sequence
                || (found == sequence
                    && seen.signed.canonical_bytes() != signed.signed.canonical_bytes());
            if replayed {
                return Err(Error::Rollback {
                    document: Document::Snapshot,
                    found,
                    highest_seen: sequence,
                });
            }
        }
        Ok(Self { signed })
    }

    pub fn from_json(
        bytes: &[u8],
        keyset: &Keyset,
        now: Timestamp,
        highest_seen: Option<(u64, &Signed<Snapshot>)>,
    ) -> Result<Self, Error> {
        Self::verify(Snapshot::from_json(bytes)?, keyset, now, highest_seen)
    }

    pub fn signed(&self) -> &Signed<Snapshot> {
        &self.signed
    }

    pub fn sequence(&self) -> u64 {
        self.signed.signed.sequence
    }

    pub fn generated_at(&self) -> Timestamp {
        self.signed.signed.generated_at
    }

    pub fn expires_at(&self) -> Timestamp {
        self.signed.signed.expires_at
    }

    /// `index_bytes` is exactly the index this snapshot names.
    pub fn check_index(&self, index_bytes: &[u8]) -> Result<(), Error> {
        let snapshot = &self.signed.signed;
        if index_bytes.len() as u64 != snapshot.index_size
            || Sha256Digest::of(index_bytes) != snapshot.index_sha256
        {
            return Err(Error::Invalid(format!(
                "index.json ({} bytes, sha256 {}) is not the index snapshot {} names ({} bytes, \
                 sha256 {})",
                index_bytes.len(),
                Sha256Digest::of(index_bytes),
                snapshot.sequence,
                snapshot.index_size,
                snapshot.index_sha256
            )));
        }
        Ok(())
    }

    /// The keyset to check releases and block lists with: `keyset` without the revoked keys.
    pub fn effective_keyset(&self, keyset: &Keyset) -> Result<Keyset, Error> {
        keyset
            .without(&self.signed.signed.revoked_keys)
            .ok_or_else(|| Error::Invalid("the snapshot revokes every trusted key".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ExtensionId, Index,
        index::test_support::release,
        signing::test_keys::{keyset, registry_key, rotation_key},
    };

    const DAY: Duration = Duration::from_secs(86_400);

    fn index_bytes() -> Vec<u8> {
        let mut index = Index::new(Timestamp(1));
        index
            .releases
            .push(Signed::sign(release("todo", "1.0.0"), &registry_key()));
        index
            .releases
            .push(Signed::sign(release("lint", "1.0.0"), &rotation_key()));
        index.to_json().expect("serializes")
    }

    fn snapshot(index: &[u8], sequence: u64, revoked: &[&str]) -> Signed<Snapshot> {
        let revoked = revoked
            .iter()
            .map(|key| KeyId::new(*key).expect("valid"))
            .collect();
        Signed::sign(
            Snapshot::for_index(index, sequence, Timestamp(1_000), DAY, revoked),
            &registry_key(),
        )
    }

    #[test]
    fn the_snapshot_pins_the_index_and_its_time() {
        let index = index_bytes();
        let verified =
            VerifiedSnapshot::verify(snapshot(&index, 3, &[]), &keyset(), Timestamp(2_000), None)
                .expect("verifies");
        let checked = Index::verify_with_snapshot(&index, &verified, &keyset()).expect("matches");
        assert_eq!(
            checked.generated_at(),
            Timestamp(1_000),
            "the signed time, not the index's"
        );
        assert_eq!(checked.extensions().count(), 2);

        // A mirror that withholds a release changes the bytes, so the snapshot no longer matches.
        let mut withheld = Index::from_json(&index).expect("parses");
        withheld.releases.pop();
        let withheld = withheld.to_json().expect("serializes");
        assert!(Index::verify_with_snapshot(&withheld, &verified, &keyset()).is_err());
    }

    #[test]
    fn expired_and_replayed_snapshots_are_refused() {
        let index = index_bytes();
        let current = snapshot(&index, 5, &[]);
        assert!(matches!(
            VerifiedSnapshot::verify(current.clone(), &keyset(), Timestamp(1_000 + 86_400), None),
            Err(Error::Expired { .. })
        ));
        let old = snapshot(&index, 4, &[]);
        assert!(matches!(
            VerifiedSnapshot::verify(old, &keyset(), Timestamp(2_000), Some((5, &current))),
            Err(Error::Rollback {
                found: 4,
                highest_seen: 5,
                ..
            })
        ));
        // The same snapshot again is fine; the same sequence with other contents is not.
        assert!(
            VerifiedSnapshot::verify(
                current.clone(),
                &keyset(),
                Timestamp(2_000),
                Some((5, &current))
            )
            .is_ok()
        );
        let conflicting = snapshot(&index, 5, &["test-rotation"]);
        assert!(matches!(
            VerifiedSnapshot::verify(
                conflicting,
                &keyset(),
                Timestamp(2_000),
                Some((5, &current))
            ),
            Err(Error::Rollback { .. })
        ));
    }

    #[test]
    fn revoked_keys_sign_nothing() {
        let index = index_bytes();
        let verified = VerifiedSnapshot::verify(
            snapshot(&index, 1, &["test-rotation"]),
            &keyset(),
            Timestamp(2_000),
            None,
        )
        .expect("verifies");
        let checked = Index::verify_with_snapshot(&index, &verified, &keyset()).expect("matches");
        assert!(
            checked
                .latest(&ExtensionId::new("todo").expect("valid"))
                .is_some()
        );
        assert!(
            checked
                .latest(&ExtensionId::new("lint").expect("valid"))
                .is_none()
        );
        assert!(matches!(
            checked.rejected()[0].error,
            Error::UnknownKey { .. }
        ));
    }

    #[test]
    fn validity_is_bounded() {
        let index = index_bytes();
        let mut long = Snapshot::for_index(&index, 1, Timestamp(0), DAY * 8, BTreeSet::new());
        assert!(long.validate().is_err());
        long.expires_at = long.generated_at;
        assert!(long.validate().is_err());
        let signed = snapshot(&index, 1, &[]);
        let json = Snapshot::to_json(&signed).expect("serializes");
        assert_eq!(Snapshot::from_json(&json), Ok(signed));
    }
}
