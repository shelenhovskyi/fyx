//! `snapshot.json`: the signed statement of what the index is right now (security-2 S6, T9).
//!
//! Releases are signed one by one, so on its own `index.json` says nothing about which releases
//! exist or how current it is: a mirror could drop a security fix, or keep serving an old index,
//! and every release would still verify. The snapshot closes that: it names the exact index by
//! size and SHA-256, carries the signed time, expires (the registry CI re-signs it at least
//! daily), and has a sequence number that never goes down. It also lists revoked keys, whose
//! signatures nothing accepts any more.
//!
//! A client keeps the last snapshot it accepted ([`VerifiedSnapshot::to_state_json`]) and passes
//! it to [`VerifiedSnapshot::verify`]; like the block list, an older one never replaces a newer
//! one.
//!
//! **Who may revoke what** (registry-repo.md, open question 3). Any snapshot key may revoke
//! `release` and `snapshot` keys. Only a snapshot signed by a key with the
//! [`KeyRole::BlocklistRevocation`] role, the offline snapshot rotation key, may revoke a
//! `blocklist` key or a key with that role; in any other snapshot such entries are ignored and
//! reported ([`VerifiedSnapshot::ignored_revocations`]). A leaked CI environment, which holds the
//! online snapshot key next to the release key, therefore cannot revoke the block-list keys and
//! unblock everything.
//!
//! **Revocations stick** (integration-5 R2). Every revocation in force carries over to every later
//! snapshot, whoever signs it, until a rotation-key snapshot leaves it out: only that key may lift
//! one. A later snapshot is checked against the keyset *without* the keys revoked so far, so a
//! leaked key that was revoked cannot sign the next snapshot and leave its own revocation out.

use std::{collections::BTreeSet, time::Duration};

use serde::{Deserialize, Serialize};

use crate::{
    Canonical, Document, Error, KeyId, KeyRole, Keyset, MAX_INDEX_SIZE, PublicKey, Sha256Digest,
    Signed, Timestamp, canonical::CanonicalWriter, check_schema, index::to_pretty_json, parse_json,
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

/// A snapshot whose signature, contents, expiry and sequence check out, with the revocations
/// it puts in force.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedSnapshot {
    signed: Signed<Snapshot>,
    /// Every key revoked now: by this snapshot, or by an earlier one and not lifted by a
    /// rotation-key snapshot since.
    revocations: BTreeSet<KeyId>,
    /// Entries of `revoked_keys` this snapshot may not revoke, for the log.
    ignored_revocations: BTreeSet<KeyId>,
}

/// What a client stores between runs: the last accepted snapshot and the revocations in force,
/// which the snapshot alone does not show once a later one left some out.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotState {
    snapshot: Signed<Snapshot>,
    #[serde(default)]
    revocations: BTreeSet<KeyId>,
    /// The field of states written before integration-5 R2: the block-list revocations only.
    /// Read, merged into `revocations`, never written.
    #[serde(default, skip_serializing)]
    protected_revocations: BTreeSet<KeyId>,
}

/// Keys only a rotation-key snapshot may revoke.
fn is_protected(key: &PublicKey) -> bool {
    key.roles.contains(&KeyRole::Blocklist) || key.roles.contains(&KeyRole::BlocklistRevocation)
}

impl VerifiedSnapshot {
    /// `keyset` is the client's trusted keyset. `previous` is the last snapshot accepted before
    /// (restored with [`VerifiedSnapshot::from_state_json`]): this one must be signed by a key it
    /// did not revoke; a lower sequence, or the same one with other contents, is a
    /// [`Error::Rollback`]; and its revocations carry over.
    pub fn verify(
        signed: Signed<Snapshot>,
        keyset: &Keyset,
        now: Timestamp,
        previous: Option<&VerifiedSnapshot>,
    ) -> Result<Self, Error> {
        match previous {
            Some(previous) => signed.verify(&previous.effective_keyset(keyset)?)?,
            None => signed.verify(keyset)?,
        }
        signed.signed.validate()?;
        if now >= signed.signed.expires_at {
            return Err(Error::Expired {
                document: Document::Snapshot,
                expires_at: signed.signed.expires_at,
            });
        }
        if let Some(previous) = previous {
            let (found, seen) = (signed.signed.sequence, previous.sequence());
            let replayed = found < seen
                || (found == seen
                    && previous.signed.signed.canonical_bytes() != signed.signed.canonical_bytes());
            if replayed {
                return Err(Error::Rollback {
                    document: Document::Snapshot,
                    found,
                    highest_seen: seen,
                });
            }
        }
        Ok(Self::with_revocations(
            signed,
            keyset,
            previous.map(|previous| &previous.revocations),
        ))
    }

    fn with_revocations(
        signed: Signed<Snapshot>,
        keyset: &Keyset,
        carried: Option<&BTreeSet<KeyId>>,
    ) -> Self {
        let may_revoke_protected = keyset
            .key(&signed.signature.key_id)
            .is_some_and(|key| key.roles.contains(&KeyRole::BlocklistRevocation));
        let (allowed, ignored_revocations): (BTreeSet<KeyId>, BTreeSet<KeyId>) = signed
            .signed
            .revoked_keys
            .iter()
            .cloned()
            .partition(|key_id| {
                may_revoke_protected || !keyset.key(key_id).is_some_and(is_protected)
            });
        // The rotation key states the whole list, so it may lift a revocation by leaving it out;
        // any other snapshot only adds to what is in force.
        let revocations = if may_revoke_protected {
            allowed
        } else {
            allowed
                .into_iter()
                .chain(carried.into_iter().flatten().cloned())
                .collect()
        };
        Self {
            signed,
            revocations,
            ignored_revocations,
        }
    }

    pub fn from_json(
        bytes: &[u8],
        keyset: &Keyset,
        now: Timestamp,
        previous: Option<&VerifiedSnapshot>,
    ) -> Result<Self, Error> {
        Self::verify(Snapshot::from_json(bytes)?, keyset, now, previous)
    }

    /// The client's stored state: this snapshot and the block-list revocations in force.
    pub fn to_state_json(&self) -> Result<Vec<u8>, Error> {
        to_pretty_json(
            &SnapshotState {
                snapshot: self.signed.clone(),
                revocations: self.revocations.clone(),
                protected_revocations: BTreeSet::new(),
            },
            Document::Snapshot,
        )
    }

    /// Restores [`VerifiedSnapshot::to_state_json`]. The signature is checked again, the expiry
    /// is not: an expired last snapshot still tells which sequence came before and which
    /// revocations are in force.
    pub fn from_state_json(bytes: &[u8], keyset: &Keyset) -> Result<Self, Error> {
        let state: SnapshotState = parse_json(bytes, Document::Snapshot)?;
        state.snapshot.verify(keyset)?;
        state.snapshot.signed.validate()?;
        let carried: BTreeSet<KeyId> = state
            .revocations
            .union(&state.protected_revocations)
            .cloned()
            .collect();
        Ok(Self::with_revocations(
            state.snapshot,
            keyset,
            Some(&carried),
        ))
    }

    /// Entries of `revoked_keys` that this snapshot was not allowed to revoke (a `blocklist` key
    /// in a snapshot not signed by the rotation key). The caller logs them.
    pub fn ignored_revocations(&self) -> &BTreeSet<KeyId> {
        &self.ignored_revocations
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

    /// The keys revoked now: this snapshot's own revocations it may make, and every earlier one
    /// still in force.
    pub fn revoked_keys(&self) -> BTreeSet<KeyId> {
        self.revocations.clone()
    }

    /// The keyset to check releases and block lists with: `keyset` without the revoked keys.
    pub fn effective_keyset(&self, keyset: &Keyset) -> Result<Keyset, Error> {
        keyset
            .without(&self.revoked_keys())
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
        let accepted = VerifiedSnapshot::verify(current.clone(), &keyset(), Timestamp(2_000), None)
            .expect("verifies");
        let old = snapshot(&index, 4, &[]);
        assert!(matches!(
            VerifiedSnapshot::verify(old, &keyset(), Timestamp(2_000), Some(&accepted)),
            Err(Error::Rollback {
                found: 4,
                highest_seen: 5,
                ..
            })
        ));
        // The same snapshot again is fine; the same sequence with other contents is not.
        assert!(
            VerifiedSnapshot::verify(current, &keyset(), Timestamp(2_000), Some(&accepted)).is_ok()
        );
        let conflicting = snapshot(&index, 5, &["test-rotation"]);
        assert!(matches!(
            VerifiedSnapshot::verify(conflicting, &keyset(), Timestamp(2_000), Some(&accepted)),
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

    /// Registry-repo.md, open question 3: the keys of the registry as it runs them.
    mod revocation {
        use super::*;
        use crate::{
            Blocklist, BlocklistCache, VerifiedBlocklist, signing::test_keys::signing_key,
        };

        fn release_key() -> crate::SigningKey {
            signing_key("rel-a", 11)
        }
        /// Online, in the release-signing environment next to the release key.
        fn snapshot_key() -> crate::SigningKey {
            signing_key("snap-a", 12)
        }
        /// Offline.
        fn snapshot_rotation_key() -> crate::SigningKey {
            signing_key("snap-b", 13)
        }
        fn blocklist_key() -> crate::SigningKey {
            signing_key("bl-a", 14)
        }
        fn blocklist_rotation_key() -> crate::SigningKey {
            signing_key("bl-b", 15)
        }

        fn registry_keyset() -> Keyset {
            Keyset::new(vec![
                release_key().public_key([KeyRole::Release]),
                snapshot_key().public_key([KeyRole::Snapshot]),
                snapshot_rotation_key()
                    .public_key([KeyRole::Snapshot, KeyRole::BlocklistRevocation]),
                blocklist_key().public_key([KeyRole::Blocklist]),
                blocklist_rotation_key().public_key([KeyRole::Blocklist]),
            ])
            .expect("valid")
        }

        fn signed_by(key: &crate::SigningKey, sequence: u64, revoked: &[&str]) -> Signed<Snapshot> {
            let revoked = revoked
                .iter()
                .map(|key| KeyId::new(*key).expect("valid"))
                .collect();
            Signed::sign(
                Snapshot::for_index(&index_bytes(), sequence, Timestamp(1_000), DAY, revoked),
                key,
            )
        }

        fn verify(
            snapshot: Signed<Snapshot>,
            previous: Option<&VerifiedSnapshot>,
        ) -> VerifiedSnapshot {
            VerifiedSnapshot::verify(snapshot, &registry_keyset(), Timestamp(2_000), previous)
                .expect("verifies")
        }

        fn blocklist_by(key: &crate::SigningKey) -> Vec<u8> {
            let list = Blocklist {
                schema: crate::BLOCKLIST_SCHEMA,
                sequence: 1,
                generated_at: Timestamp(1_000),
                expires_at: Timestamp(1_000 + 86_400),
                entries: Vec::new(),
            };
            serde_json::to_vec(&Signed::sign(list, key)).expect("serializes")
        }

        fn blocklist_key_works(snapshot: &VerifiedSnapshot) -> bool {
            let keyset = snapshot
                .effective_keyset(&registry_keyset())
                .expect("keys left");
            VerifiedBlocklist::from_json(&blocklist_by(&blocklist_key()), &keyset).is_ok()
        }

        /// A leaked CI environment signs a snapshot that revokes both block-list keys: the
        /// revocation is ignored, and the block list keeps verifying.
        #[test]
        fn the_online_snapshot_key_cannot_revoke_block_list_keys() {
            let attack = verify(
                signed_by(&snapshot_key(), 7, &["bl-a", "bl-b", "snap-b", "rel-a"]),
                None,
            );
            assert_eq!(
                attack.ignored_revocations(),
                &["bl-a", "bl-b", "snap-b"]
                    .map(|key| KeyId::new(key).expect("valid"))
                    .into()
            );
            assert!(blocklist_key_works(&attack));
            // Release keys it may revoke.
            assert_eq!(
                attack.revoked_keys(),
                [KeyId::new("rel-a").expect("valid")].into()
            );
            let cache = BlocklistCache::apply_fetch(
                None,
                Ok(&blocklist_by(&blocklist_key())),
                &attack.effective_keyset(&registry_keyset()).expect("keys"),
                Timestamp(2_000),
            );
            assert!(matches!(cache, crate::CacheUpdate::Replace(_)));
        }

        /// The emergency snapshot, signed offline by the snapshot rotation key, revokes a leaked
        /// block-list key; the next daily snapshots, signed by the online key, keep it revoked,
        /// and only another rotation-key snapshot lifts it.
        #[test]
        fn the_offline_rotation_key_can_and_the_revocation_carries_over() {
            let emergency = verify(signed_by(&snapshot_rotation_key(), 8, &["bl-a"]), None);
            assert!(emergency.ignored_revocations().is_empty());
            assert!(!blocklist_key_works(&emergency));
            let keyset = emergency
                .effective_keyset(&registry_keyset())
                .expect("keys");
            assert!(
                VerifiedBlocklist::from_json(&blocklist_by(&blocklist_rotation_key()), &keyset)
                    .is_ok(),
                "the promoted rotation key signs block lists"
            );

            let daily = verify(signed_by(&snapshot_key(), 9, &[]), Some(&emergency));
            assert!(!blocklist_key_works(&daily), "carried over");
            // Across a restart too.
            let state = daily.to_state_json().expect("serializes");
            let restored =
                VerifiedSnapshot::from_state_json(&state, &registry_keyset()).expect("restores");
            assert_eq!(restored, daily);
            let next = verify(signed_by(&snapshot_key(), 10, &["bl-b"]), Some(&restored));
            assert!(!blocklist_key_works(&next));
            assert!(
                next.ignored_revocations()
                    .contains(&KeyId::new("bl-b").expect("valid"))
            );

            let lifted = verify(signed_by(&snapshot_rotation_key(), 11, &[]), Some(&next));
            assert!(blocklist_key_works(&lifted));
        }

        /// integration-5 R2: the leaked online snapshot key is revoked; the attacker signs the
        /// next sequence with it, leaving the revocation out. Refused: the revoked key signs
        /// nothing after its revocation. A second online key carries the revocation on, and
        /// leaving it out does not lift it.
        #[test]
        fn a_revoked_snapshot_key_cannot_undo_its_revocation() {
            let second = signing_key("snap-c", 16);
            let keyset = Keyset::new(vec![
                release_key().public_key([KeyRole::Release]),
                snapshot_key().public_key([KeyRole::Snapshot]),
                second.public_key([KeyRole::Snapshot]),
                snapshot_rotation_key()
                    .public_key([KeyRole::Snapshot, KeyRole::BlocklistRevocation]),
                blocklist_key().public_key([KeyRole::Blocklist]),
            ])
            .expect("valid");
            let verify = |snapshot, previous| {
                VerifiedSnapshot::verify(snapshot, &keyset, Timestamp(2_000), previous)
            };
            let snap_a = KeyId::new("snap-a").expect("valid");
            let revoking = verify(signed_by(&second, 8, &["snap-a"]), None).expect("verifies");
            assert!(revoking.revoked_keys().contains(&snap_a));

            let attack = verify(signed_by(&snapshot_key(), 9, &[]), Some(&revoking));
            assert!(
                matches!(attack, Err(Error::UnknownKey { .. })),
                "a revoked key signs no snapshot: {attack:?}"
            );
            // Across a restart too.
            let state = revoking.to_state_json().expect("serializes");
            let restored = VerifiedSnapshot::from_state_json(&state, &keyset).expect("restores");
            assert!(verify(signed_by(&snapshot_key(), 9, &[]), Some(&restored)).is_err());

            let daily = verify(signed_by(&second, 9, &[]), Some(&restored)).expect("verifies");
            assert!(
                daily.revoked_keys().contains(&snap_a),
                "leaving a revocation out does not lift it"
            );
            let release = KeyId::new("rel-a").expect("valid");
            let more = verify(signed_by(&second, 10, &["rel-a"]), Some(&daily)).expect("verifies");
            assert_eq!(more.revoked_keys(), [snap_a.clone(), release].into());

            // Only the rotation key lifts one.
            let lifted = verify(
                signed_by(&snapshot_rotation_key(), 11, &["rel-a"]),
                Some(&more),
            )
            .expect("verifies");
            assert_eq!(
                lifted.revoked_keys(),
                [KeyId::new("rel-a").expect("valid")].into()
            );
            assert!(verify(signed_by(&snapshot_key(), 12, &[]), Some(&lifted)).is_ok());
        }

        /// A state written before R2 (`protected_revocations` only) still restores, and its
        /// revocations stay in force.
        #[test]
        fn old_states_restore() {
            let emergency = verify(signed_by(&snapshot_rotation_key(), 8, &["bl-a"]), None);
            let state = serde_json::json!({
                "snapshot": emergency.signed(),
                "protected_revocations": ["bl-a"],
            });
            let restored = VerifiedSnapshot::from_state_json(
                &serde_json::to_vec(&state).expect("serializes"),
                &registry_keyset(),
            )
            .expect("restores");
            assert!(!blocklist_key_works(&restored));
            let written: serde_json::Value =
                serde_json::from_slice(&restored.to_state_json().expect("serializes"))
                    .expect("json");
            assert!(written.get("protected_revocations").is_none());
            assert_eq!(written["revocations"], serde_json::json!(["bl-a"]));
        }
    }
}
