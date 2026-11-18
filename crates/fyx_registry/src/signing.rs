//! Ed25519 keys, keysets and signatures.
//!
//! A signature names the key that made it (`key_id`). A client holds a [`Keyset`]: the registry
//! key and one rotation key are compiled in (decision 18), so the registry can move to the
//! rotation key without a client release that changes how verification works.
//!
//! Every key has roles (security-2 T9): a key may only sign the documents of its roles, so a
//! leaked block-list key cannot publish a release. A signed snapshot can revoke keys
//! ([`crate::VerifiedSnapshot::effective_keyset`]).

use std::{collections::BTreeSet, fmt};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use ring::signature::{ED25519, Ed25519KeyPair, KeyPair as _, UnparsedPublicKey};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use zeroize::{Zeroize as _, Zeroizing};

use crate::{Document, Error};

const MAX_KEY_ID_LENGTH: usize = 64;

/// The name of a signing key, e.g. `fyx-2026-a`: lowercase ASCII letters, digits, `-` and `.`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyId(String);

impl KeyId {
    pub fn new(value: impl Into<String>) -> Result<Self, Error> {
        let value = value.into();
        let valid_characters = value
            .chars()
            .all(|character| matches!(character, 'a'..='z' | '0'..='9' | '-' | '.'));
        let starts_alphanumeric = value
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric());
        if value.len() > MAX_KEY_ID_LENGTH || !valid_characters || !starts_alphanumeric {
            return Err(Error::Invalid(format!(
                "key id {value:?} must be 1 to {MAX_KEY_ID_LENGTH} characters of a-z, 0-9, '-' \
                 and '.', starting with a letter or digit"
            )));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for KeyId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for KeyId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for KeyId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

/// A signature over a document's canonical bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Signature {
    pub key_id: KeyId,
    #[serde(with = "base64_array")]
    pub ed25519: [u8; 64],
}

/// What a key may sign.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyRole {
    /// One release of one extension.
    Release,
    /// `blocklist.json`.
    Blocklist,
    /// `snapshot.json`: the index as a whole, and key revocations.
    Snapshot,
    /// Not a document: a snapshot signed by a key with this role may revoke `blocklist` keys,
    /// and keys with this role. Only the offline snapshot rotation key has it (registry-repo.md,
    /// open question 3), so the online snapshot key, which sits next to the release key in CI,
    /// cannot turn the block list off.
    #[serde(rename = "blocklist-revocation")]
    BlocklistRevocation,
}

impl KeyRole {
    pub const ALL: [KeyRole; 4] = [
        KeyRole::Release,
        KeyRole::Blocklist,
        KeyRole::Snapshot,
        KeyRole::BlocklistRevocation,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            KeyRole::Release => "release",
            KeyRole::Blocklist => "blocklist",
            KeyRole::Snapshot => "snapshot",
            KeyRole::BlocklistRevocation => "blocklist-revocation",
        }
    }
}

impl fmt::Display for KeyRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A public key, its id and its roles, as listed in a keyset file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicKey {
    pub key_id: KeyId,
    #[serde(with = "base64_array")]
    pub ed25519: [u8; 32],
    /// At least one; a key signs nothing outside its roles.
    pub roles: BTreeSet<KeyRole>,
}

/// The public keys a client or the registry CI accepts signatures from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Keyset {
    keys: Vec<PublicKey>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeysetFile {
    keys: Vec<PublicKey>,
}

impl Keyset {
    pub fn new(keys: Vec<PublicKey>) -> Result<Self, Error> {
        if keys.is_empty() {
            return Err(Error::Invalid("a keyset needs at least one key".into()));
        }
        for (index, key) in keys.iter().enumerate() {
            if key.roles.is_empty() {
                return Err(Error::Invalid(format!("key {} has no roles", key.key_id)));
            }
            if keys[..index].iter().any(|other| other.key_id == key.key_id) {
                return Err(Error::Invalid(format!(
                    "key id {} appears twice in the keyset",
                    key.key_id
                )));
            }
        }
        Ok(Self { keys })
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, Error> {
        let file: KeysetFile = serde_json::from_slice(bytes).map_err(|error| Error::Parse {
            document: Document::Keyset,
            message: error.to_string(),
        })?;
        Self::new(file.keys)
    }

    pub fn keys(&self) -> &[PublicKey] {
        &self.keys
    }

    pub fn key(&self, key_id: &KeyId) -> Option<&PublicKey> {
        self.keys.iter().find(|key| &key.key_id == key_id)
    }

    /// This keyset without the revoked keys; `None` if no key would be left.
    pub fn without(&self, revoked: &BTreeSet<KeyId>) -> Option<Self> {
        let keys: Vec<PublicKey> = self
            .keys
            .iter()
            .filter(|key| !revoked.contains(&key.key_id))
            .cloned()
            .collect();
        (!keys.is_empty()).then_some(Self { keys })
    }

    /// The registry's role separation (registry-repo.md, "Keys and roles"), for the keyset the
    /// registry repository publishes. Clients do not apply it: they only check that a signature
    /// comes from a key with the right role.
    ///
    /// Refuses a key with two signing roles (`release`, `blocklist`, `snapshot`), and
    /// `blocklist-revocation` on a key without `snapshot`. With `online_snapshot_key`, the key
    /// the CI signs snapshots with (`SNAPSHOT_KEY`), also refuses that key unless it is in the
    /// keyset with `snapshot` and without `blocklist-revocation`: a rotation must not put the
    /// role that may revoke block-list keys into CI by accident.
    pub fn check_role_separation(&self, online_snapshot_key: Option<&KeyId>) -> Result<(), Error> {
        for key in &self.keys {
            let signing_roles: Vec<&str> = key
                .roles
                .iter()
                .filter(|role| **role != KeyRole::BlocklistRevocation)
                .map(|role| role.as_str())
                .collect();
            if signing_roles.len() > 1 {
                return Err(Error::Invalid(format!(
                    "key {} has more than one signing role: {}",
                    key.key_id,
                    signing_roles.join(", ")
                )));
            }
            if key.roles.contains(&KeyRole::BlocklistRevocation)
                && !key.roles.contains(&KeyRole::Snapshot)
            {
                return Err(Error::Invalid(format!(
                    "key {} has blocklist-revocation but not snapshot; only the offline snapshot                      rotation key may have it",
                    key.key_id
                )));
            }
        }
        if let Some(key_id) = online_snapshot_key {
            let key = self.key(key_id).ok_or_else(|| {
                Error::Invalid(format!(
                    "the online snapshot key {key_id} is not in the keyset"
                ))
            })?;
            if !key.roles.contains(&KeyRole::Snapshot) {
                return Err(Error::Invalid(format!(
                    "the online snapshot key {key_id} has no snapshot role"
                )));
            }
            if key.roles.contains(&KeyRole::BlocklistRevocation) {
                return Err(Error::Invalid(format!(
                    "the online snapshot key {key_id} has blocklist-revocation; that role                      belongs to the offline rotation key only"
                )));
            }
        }
        Ok(())
    }

    /// Checks a signature by a key of this keyset that has `role`.
    pub fn verify(
        &self,
        message: &[u8],
        signature: &Signature,
        role: KeyRole,
    ) -> Result<(), Error> {
        let key = self
            .keys
            .iter()
            .find(|key| key.key_id == signature.key_id)
            .ok_or_else(|| Error::UnknownKey {
                key_id: signature.key_id.to_string(),
            })?;
        if !key.roles.contains(&role) {
            return Err(Error::WrongRole {
                key_id: signature.key_id.to_string(),
                role,
            });
        }
        UnparsedPublicKey::new(&ED25519, key.ed25519)
            .verify(message, &signature.ed25519)
            .map_err(|_| Error::BadSignature {
                key_id: signature.key_id.to_string(),
            })
    }
}

/// A private signing key. Only the registry CI holds one; it is read from a file or an
/// environment variable and never written by this crate.
pub struct SigningKey {
    key_id: KeyId,
    key_pair: Ed25519KeyPair,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SigningKeyFile {
    key_id: KeyId,
    seed: String,
}

impl SigningKey {
    /// Any 32 bytes are a valid Ed25519 seed (RFC 8032 §5.1.5).
    pub fn from_seed(key_id: KeyId, seed: &[u8; 32]) -> Result<Self, Error> {
        let key_pair = Ed25519KeyPair::from_seed_unchecked(seed)
            .map_err(|error| Error::Invalid(format!("signing key {key_id}: {error}")))?;
        Ok(Self { key_id, key_pair })
    }

    /// Reads `{"key_id": "...", "seed": "<base64 of 32 bytes>"}`.
    pub fn from_json(bytes: &[u8]) -> Result<Self, Error> {
        let mut file: SigningKeyFile = serde_json::from_slice(bytes).map_err(|_| Error::Parse {
            document: Document::SigningKey,
            // serde's message can quote the input, which is a secret.
            message: "expected {\"key_id\": \"...\", \"seed\": \"<base64>\"}".into(),
        })?;
        let decoded = BASE64.decode(file.seed.as_bytes());
        file.seed.zeroize();
        let decoded = Zeroizing::new(decoded.map_err(|_| {
            Error::Invalid(format!(
                "signing key {}: the seed is not base64",
                file.key_id
            ))
        })?);
        let seed: Zeroizing<[u8; 32]> =
            Zeroizing::new(decoded.as_slice().try_into().map_err(|_| {
                Error::Invalid(format!(
                    "signing key {}: the seed must be 32 bytes",
                    file.key_id
                ))
            })?);
        Self::from_seed(file.key_id, &seed)
    }

    pub fn key_id(&self) -> &KeyId {
        &self.key_id
    }

    /// The public key entry for a keyset file, with the roles it is trusted for.
    pub fn public_key(&self, roles: impl IntoIterator<Item = KeyRole>) -> PublicKey {
        let mut ed25519 = [0; 32];
        ed25519.copy_from_slice(self.key_pair.public_key().as_ref());
        PublicKey {
            key_id: self.key_id.clone(),
            ed25519,
            roles: roles.into_iter().collect(),
        }
    }

    pub fn sign(&self, message: &[u8]) -> Signature {
        let mut ed25519 = [0; 64];
        ed25519.copy_from_slice(self.key_pair.sign(message).as_ref());
        Signature {
            key_id: self.key_id.clone(),
            ed25519,
        }
    }
}

impl fmt::Debug for SigningKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SigningKey")
            .field("key_id", &self.key_id)
            .finish_non_exhaustive()
    }
}

pub(crate) mod base64_array {
    use super::*;

    pub fn serialize<S: Serializer, const N: usize>(
        bytes: &[u8; N],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&BASE64.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>, const N: usize>(
        deserializer: D,
    ) -> Result<[u8; N], D::Error> {
        let text = String::deserialize(deserializer)?;
        let bytes = BASE64.decode(text.as_bytes()).map_err(de::Error::custom)?;
        bytes
            .try_into()
            .map_err(|bytes: Vec<u8>| de::Error::invalid_length(bytes.len(), &"base64 of N bytes"))
    }
}

/// Deterministic keys for tests. They are public by construction and must never sign anything
/// outside a test.
#[cfg(test)]
pub(crate) mod test_keys {
    use super::*;

    pub(crate) fn signing_key(name: &str, seed_byte: u8) -> SigningKey {
        let key_id = KeyId::new(name).expect("test key ids are valid");
        SigningKey::from_seed(key_id, &[seed_byte; 32]).expect("any seed is valid")
    }

    pub(crate) fn registry_key() -> SigningKey {
        signing_key("test-registry", 1)
    }

    pub(crate) fn rotation_key() -> SigningKey {
        signing_key("test-rotation", 2)
    }

    pub(crate) fn keyset() -> Keyset {
        Keyset::new(vec![
            registry_key().public_key(KeyRole::ALL),
            rotation_key().public_key(KeyRole::ALL),
        ])
        .expect("distinct key ids")
    }
}

#[cfg(test)]
mod tests {
    use super::{test_keys::*, *};
    use proptest::prelude::*;

    #[test]
    fn rfc8032_test_vector_1() {
        // RFC 8032 §7.1, TEST 1: empty message.
        let seed: [u8; 32] =
            hex::decode("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60")
                .expect("hex")
                .try_into()
                .expect("32 bytes");
        let key = SigningKey::from_seed(KeyId::new("rfc").expect("valid"), &seed).expect("seed");
        assert_eq!(
            hex::encode(key.public_key(KeyRole::ALL).ed25519),
            "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
        );
        assert_eq!(
            hex::encode(key.sign(b"").ed25519),
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155\
             5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b"
        );
    }

    #[test]
    fn verifies_with_either_key_of_the_keyset() {
        let keyset = keyset();
        for key in [registry_key(), rotation_key()] {
            let signature = key.sign(b"message");
            assert_eq!(
                keyset.verify(b"message", &signature, KeyRole::Release),
                Ok(())
            );
        }
    }

    #[test]
    fn refuses_unknown_and_mislabelled_keys() {
        let keyset = keyset();
        let stranger = signing_key("stranger", 9);
        assert_eq!(
            keyset.verify(b"message", &stranger.sign(b"message"), KeyRole::Release),
            Err(Error::UnknownKey {
                key_id: "stranger".into()
            })
        );

        let mut mislabelled = rotation_key().sign(b"message");
        mislabelled.key_id = registry_key().key_id().clone();
        assert_eq!(
            keyset.verify(b"message", &mislabelled, KeyRole::Release),
            Err(Error::BadSignature {
                key_id: "test-registry".into()
            })
        );
    }

    #[test]
    fn keyset_file_round_trips_and_rejects_duplicates() {
        let json = serde_json::to_vec(&keyset()).expect("serializes");
        assert_eq!(Keyset::from_json(&json), Ok(keyset()));

        let duplicate = Keyset::new(vec![
            registry_key().public_key(KeyRole::ALL),
            registry_key().public_key(KeyRole::ALL),
        ]);
        assert!(duplicate.is_err());
        assert!(Keyset::new(vec![registry_key().public_key([])]).is_err());
        assert!(Keyset::from_json(br#"{"keys": []}"#).is_err());
        assert!(Keyset::from_json(br#"{"keys": [], "extra": 1}"#).is_err());
    }

    #[test]
    fn role_separation() {
        use KeyRole::*;
        let key = |name: &str, seed: u8, roles: &[KeyRole]| {
            signing_key(name, seed).public_key(roles.iter().copied())
        };
        let id = |name: &str| KeyId::new(name).expect("valid");
        // The layout registry-repo.md describes.
        let registry = Keyset::new(vec![
            key("release", 1, &[Release]),
            key("blocklist", 2, &[Blocklist]),
            key("snap-a", 3, &[Snapshot]),
            key("snap-b", 4, &[Snapshot, BlocklistRevocation]),
        ])
        .expect("keyset");
        assert_eq!(registry.check_role_separation(None), Ok(()));
        assert_eq!(registry.check_role_separation(Some(&id("snap-a"))), Ok(()));

        // A rotation that makes the rotation key the online one, without a new rotation key.
        let error = registry
            .check_role_separation(Some(&id("snap-b")))
            .unwrap_err();
        assert!(
            error.to_string().contains("blocklist-revocation"),
            "{error}"
        );
        for online in ["release", "missing"] {
            assert!(registry.check_role_separation(Some(&id(online))).is_err());
        }

        for roles in [
            &[Release, Snapshot][..],
            &[Release, Blocklist],
            &[Blocklist, Snapshot, BlocklistRevocation],
            &[BlocklistRevocation],
            &[Blocklist, BlocklistRevocation],
        ] {
            let keyset = Keyset::new(vec![key("snap-a", 3, &[Snapshot]), key("k", 1, roles)])
                .expect("keyset");
            assert!(
                keyset.check_role_separation(None).is_err(),
                "{roles:?} should be refused"
            );
        }
        // The test keyset, with every role on both keys, is not a registry keyset.
        assert!(keyset().check_role_separation(None).is_err());
    }

    #[test]
    fn signing_key_file_parses_without_echoing_the_seed() {
        let seed = BASE64.encode([7u8; 32]);
        let json = format!(r#"{{"key_id": "ci-2026", "seed": "{seed}"}}"#);
        let key = SigningKey::from_json(json.as_bytes()).expect("valid key file");
        assert_eq!(key.key_id().as_str(), "ci-2026");
        assert!(!format!("{key:?}").contains(&seed));

        let short = format!(
            r#"{{"key_id": "ci", "seed": "{}"}}"#,
            BASE64.encode([7u8; 31])
        );
        assert!(SigningKey::from_json(short.as_bytes()).is_err());

        let error = SigningKey::from_json(br#"{"key_id": "ci", "seed": 12345678}"#)
            .expect_err("seed must be a string");
        assert!(!error.to_string().contains("12345678"));
    }

    #[test]
    fn key_ids_are_restricted() {
        for valid in ["a", "fyx-2026-a", "0.1", "x".repeat(64).as_str()] {
            assert!(KeyId::new(valid).is_ok(), "{valid}");
        }
        for invalid in ["", "-a", ".a", "A", "a b", "a/b", "x".repeat(65).as_str()] {
            assert!(KeyId::new(invalid).is_err(), "{invalid}");
        }
    }

    proptest! {
        #[test]
        fn any_flipped_message_bit_is_refused(
            message in proptest::collection::vec(any::<u8>(), 1..256),
            position in any::<prop::sample::Index>(),
            bit in 0u8..8,
        ) {
            let keyset = keyset();
            let signature = registry_key().sign(&message);
            let mut tampered = message.clone();
            tampered[position.index(message.len())] ^= 1 << bit;
            prop_assert!(keyset.verify(&tampered, &signature, KeyRole::Release).is_err());
        }

        #[test]
        fn any_flipped_signature_bit_is_refused(
            message in proptest::collection::vec(any::<u8>(), 0..256),
            position in 0usize..64,
            bit in 0u8..8,
        ) {
            let keyset = keyset();
            let mut signature = registry_key().sign(&message);
            signature.ed25519[position] ^= 1 << bit;
            prop_assert!(keyset.verify(&message, &signature, KeyRole::Release).is_err());
        }
    }

    #[test]
    fn keys_sign_only_for_their_roles() {
        let keyset = Keyset::new(vec![
            registry_key().public_key([KeyRole::Release]),
            rotation_key().public_key([KeyRole::Blocklist, KeyRole::Snapshot]),
        ])
        .expect("valid");
        let release = registry_key().sign(b"m");
        assert_eq!(keyset.verify(b"m", &release, KeyRole::Release), Ok(()));
        assert_eq!(
            keyset.verify(b"m", &release, KeyRole::Blocklist),
            Err(Error::WrongRole {
                key_id: "test-registry".into(),
                role: KeyRole::Blocklist
            })
        );

        let revoked = [registry_key().key_id().clone()].into();
        let remaining = keyset.without(&revoked).expect("one key left");
        assert!(matches!(
            remaining.verify(b"m", &release, KeyRole::Release),
            Err(Error::UnknownKey { .. })
        ));
        let all = [
            registry_key().key_id().clone(),
            rotation_key().key_id().clone(),
        ]
        .into();
        assert_eq!(keyset.without(&all), None);
    }
}
