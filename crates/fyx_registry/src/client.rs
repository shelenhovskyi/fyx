//! The registry client core (platform plan 3.3): what the extensions page and the install store
//! ask the registry, without any I/O of its own. The host supplies a [`Fetch`] (HTTP in the
//! product, a map in tests), the clock (`now`), and the bytes it stored last time
//! ([`StoredState`]); this module decides what to trust and what to install.
//!
//! Order of trust (registry-format.md, "snapshot.json"):
//! 1. the snapshot, against the compiled-in keyset and the last accepted snapshot (expiry,
//!    sequence, revoked keys);
//! 2. the index the snapshot names, by size and SHA-256, its releases against the snapshot's
//!    keyset (revoked keys removed);
//! 3. the block list, against the same keyset, through [`BlocklistCache::apply_fetch`].
//!
//! A refused or expired snapshot stops installs and updates until a good one arrives; the block
//! list keeps applying, so blocked versions are still rolled back or disabled. Being offline
//! stops nothing while the last snapshot is valid.

use std::future::Future;

use crate::update::Additions;
use crate::{
    BlocklistCache, CacheUpdate, Error, ExtensionId, Freshness, Index, InstallRefusal,
    InstalledExtension, Keyset, MAX_BLOCKLIST_SIZE, Permissions, PlannedAction, Release, Timestamp,
    VerifiedBlocklist, VerifiedIndex, VerifiedSnapshot, plan_install, plan_updates,
    snapshot::MAX_SNAPSHOT_SIZE,
};

/// A registry file the client asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistryFile {
    /// `snapshot.json`.
    Snapshot,
    /// `index.json`.
    Index,
    /// `blocklist.json`.
    Blocklist,
    /// A release's `.fyxext`, at `package.url` of the release.
    Package { url: String },
}

/// One fetch: the file, and the most bytes the client will accept. A fetcher should stop reading
/// past `max_bytes`; the client refuses a longer body anyway.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchRequest {
    pub file: RegistryFile,
    pub max_bytes: u64,
}

/// How the client gets registry files. The host implements it (HTTP from the registry's base
/// URL, plan 3.5); the client never talks to the network itself.
pub trait Fetch {
    /// The body of `request.file`. Any error (network, HTTP status, a body over
    /// `request.max_bytes`) is reported as is; the client treats them all as "could not fetch".
    fn fetch(&self, request: &FetchRequest)
    -> impl Future<Output = anyhow::Result<Vec<u8>>> + Send;
}

/// What the host stores between runs, as opaque bytes (one file each). Every part is verified
/// again by [`RegistryClient::restore`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StoredState {
    /// [`VerifiedSnapshot::to_state_json`] of the last accepted snapshot.
    pub snapshot: Option<Vec<u8>>,
    /// The `index.json` that snapshot names.
    pub index: Option<Vec<u8>>,
    /// [`BlocklistCache::to_json`].
    pub blocklist: Option<Vec<u8>>,
}

/// Why installs and updates are stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unavailable {
    /// No snapshot and index were ever accepted.
    NeverFetched,
    /// The last accepted snapshot expired: the mirror may be frozen. Shown until a fresh one
    /// arrives.
    Expired { expires_at: Timestamp },
    /// The last fetched snapshot or index failed verification (a rollback, a bad signature, an
    /// index the snapshot does not name). Cleared by the next good refresh.
    Refused(Error),
}

/// The outcome of [`RegistryClient::refresh_index`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IndexRefresh {
    /// A newer snapshot and its index were accepted; store [`RegistryClient::stored_state`].
    Updated,
    /// The snapshot is the one already accepted, or a re-signed one naming the same index.
    Unchanged,
    /// Nothing could be fetched; the state is as before (offline stops nothing by itself).
    FetchFailed(String),
    /// The fetched snapshot or index was refused; installs and updates stop
    /// ([`Unavailable::Refused`]).
    Refused(Error),
}

/// A fresh install the user can confirm.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallPlan<'a> {
    pub release: &'a Release,
    /// What the install confirmation asks for: T1 and T2 capabilities, network domains,
    /// programs and the tier. Empty for an all-T0 extension, which installs without asking.
    pub consent: Additions,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallError {
    Unavailable(Unavailable),
    Refused(InstallRefusal),
}

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("could not download the package: {0:#}")]
    Fetch(anyhow::Error),
    #[error(transparent)]
    Mismatch(Error),
}

struct AcceptedIndex {
    bytes: Vec<u8>,
    verified: VerifiedIndex,
}

/// The client's view of the registry; see the module documentation.
pub struct RegistryClient {
    /// The compiled-in keyset.
    keyset: Keyset,
    snapshot: Option<VerifiedSnapshot>,
    index: Option<AcceptedIndex>,
    blocklist: Option<BlocklistCache>,
    refused: Option<Error>,
    /// An empty index, for planning while installs and updates are stopped.
    empty: VerifiedIndex,
}

impl RegistryClient {
    /// A client with nothing stored yet.
    pub fn new(keyset: Keyset) -> Self {
        let empty = Index::new(Timestamp(0)).verify(&keyset);
        Self {
            keyset,
            snapshot: None,
            index: None,
            blocklist: None,
            refused: None,
            empty,
        }
    }

    /// Restores what [`RegistryClient::stored_state`] gave last time. A part that no longer
    /// verifies (a key this build dropped, a damaged file) is left out and reported; the next
    /// refresh fetches it again. The index is kept only with the snapshot that names it.
    pub fn restore(keyset: Keyset, stored: &StoredState) -> (Self, Vec<Error>) {
        let mut client = Self::new(keyset);
        let mut problems = Vec::new();
        if let Some(bytes) = &stored.snapshot {
            match VerifiedSnapshot::from_state_json(bytes, &client.keyset) {
                Ok(snapshot) => client.snapshot = Some(snapshot),
                Err(error) => problems.push(error),
            }
        }
        if let (Some(snapshot), Some(bytes)) = (&client.snapshot, &stored.index) {
            match Index::verify_with_snapshot(bytes, snapshot, &client.keyset) {
                Ok(verified) => {
                    client.index = Some(AcceptedIndex {
                        bytes: bytes.clone(),
                        verified,
                    })
                }
                Err(error) => problems.push(error),
            }
        }
        if let Some(bytes) = &stored.blocklist {
            match client
                .blocklist_keyset()
                .and_then(|keyset| BlocklistCache::from_json(bytes, &keyset))
            {
                Ok(cache) => client.blocklist = Some(cache),
                Err(error) => problems.push(error),
            }
        }
        (client, problems)
    }

    /// What to store now. Store it after every [`IndexRefresh::Updated`] and every block-list
    /// `Replace` or `Refresh`.
    pub fn stored_state(&self) -> Result<StoredState, Error> {
        Ok(StoredState {
            snapshot: self
                .snapshot
                .as_ref()
                .map(VerifiedSnapshot::to_state_json)
                .transpose()?,
            index: self.index.as_ref().map(|index| index.bytes.clone()),
            blocklist: self
                .blocklist
                .as_ref()
                .map(BlocklistCache::to_json)
                .transpose()?,
        })
    }

    /// Fetches and verifies the snapshot, then the index it names (skipped when it names the
    /// index already held). Both are accepted together or not at all.
    pub async fn refresh_index(&mut self, fetch: &impl Fetch, now: Timestamp) -> IndexRefresh {
        let request = FetchRequest {
            file: RegistryFile::Snapshot,
            max_bytes: MAX_SNAPSHOT_SIZE as u64,
        };
        let bytes = match fetch_capped(fetch, &request).await {
            Ok(bytes) => bytes,
            Err(error) => return IndexRefresh::FetchFailed(format!("{error:#}")),
        };
        let snapshot =
            match VerifiedSnapshot::from_json(&bytes, &self.keyset, now, self.snapshot.as_ref()) {
                Ok(snapshot) => snapshot,
                Err(error) => return self.refuse(error),
            };
        let same_index = self
            .index
            .as_ref()
            .is_some_and(|index| snapshot.check_index(&index.bytes).is_ok());
        // The held index is kept until the new pair is accepted.
        let held = if same_index { self.index.take() } else { None };
        let index_bytes = if let Some(held) = &held {
            held.bytes.clone()
        } else {
            let request = FetchRequest {
                file: RegistryFile::Index,
                max_bytes: snapshot.signed().signed.index_size,
            };
            match fetch_capped(fetch, &request).await {
                Ok(bytes) => bytes,
                Err(error) => return IndexRefresh::FetchFailed(format!("{error:#}")),
            }
        };
        let verified = match Index::verify_with_snapshot(&index_bytes, &snapshot, &self.keyset) {
            Ok(verified) => verified,
            Err(error) => {
                self.index = held;
                return self.refuse(error);
            }
        };
        let unchanged = same_index
            && self.snapshot.as_ref().is_some_and(|previous| {
                previous.sequence() == snapshot.sequence()
                    && previous.revoked_keys() == snapshot.revoked_keys()
            });
        self.snapshot = Some(snapshot);
        self.index = Some(AcceptedIndex {
            bytes: index_bytes,
            verified,
        });
        self.refused = None;
        self.recheck_blocklist();
        if unchanged {
            IndexRefresh::Unchanged
        } else {
            IndexRefresh::Updated
        }
    }

    /// Fetches the block list and applies [`BlocklistCache::apply_fetch`] with the keyset of the
    /// last accepted snapshot. The host stores the state on `Replace` and `Refresh`, and logs
    /// `Keep`. Due every 6 h ([`BlocklistCache::fetch_due`]).
    pub async fn refresh_blocklist(&mut self, fetch: &impl Fetch, now: Timestamp) -> CacheUpdate {
        let keyset = match self.blocklist_keyset() {
            Ok(keyset) => keyset,
            Err(error) => return CacheUpdate::Keep(crate::KeepReason::Invalid(error)),
        };
        let request = FetchRequest {
            file: RegistryFile::Blocklist,
            max_bytes: MAX_BLOCKLIST_SIZE as u64,
        };
        let fetched = fetch_capped(fetch, &request)
            .await
            .map_err(|error| format!("{error:#}"));
        let update = BlocklistCache::apply_fetch(
            self.blocklist.as_ref(),
            fetched.as_deref().map_err(Clone::clone),
            &keyset,
            now,
        );
        if let CacheUpdate::Replace(cache) | CacheUpdate::Refresh(cache) = &update {
            self.blocklist = Some(cache.clone());
        }
        update
    }

    /// The verified index, unless installs and updates are stopped.
    pub fn index(&self, now: Timestamp) -> Result<&VerifiedIndex, Unavailable> {
        if let Some(error) = &self.refused {
            return Err(Unavailable::Refused(error.clone()));
        }
        let (Some(snapshot), Some(index)) = (&self.snapshot, &self.index) else {
            return Err(Unavailable::NeverFetched);
        };
        if now >= snapshot.expires_at() {
            return Err(Unavailable::Expired {
                expires_at: snapshot.expires_at(),
            });
        }
        Ok(&index.verified)
    }

    /// The last accepted snapshot, for the extensions page and the log
    /// (`ignored_revocations`).
    pub fn snapshot(&self) -> Option<&VerifiedSnapshot> {
        self.snapshot.as_ref()
    }

    pub fn blocklist(&self) -> Option<&VerifiedBlocklist> {
        self.blocklist.as_ref().map(|cache| &cache.blocklist)
    }

    pub fn blocklist_cache(&self) -> Option<&BlocklistCache> {
        self.blocklist.as_ref()
    }

    /// For the "Block list is N days old" warning.
    pub fn blocklist_freshness(&self, now: Timestamp) -> Freshness {
        BlocklistCache::freshness(self.blocklist.as_ref(), now)
    }

    /// A fresh install of `id`: the newest release that is not blocked and passes the age gate,
    /// and what the install confirmation must ask for.
    pub fn plan_install(
        &self,
        id: &ExtensionId,
        now: Timestamp,
    ) -> Result<InstallPlan<'_>, InstallError> {
        let index = self.index(now).map_err(InstallError::Unavailable)?;
        let release =
            plan_install(id, index, self.blocklist(), now).map_err(InstallError::Refused)?;
        Ok(InstallPlan {
            release,
            consent: Permissions::default().additions(release),
        })
    }

    /// Actions for the installed extensions ([`plan_updates`]). While installs and updates are
    /// stopped, only the block list acts: blocked versions are still rolled back or disabled, and
    /// nothing updates or shows as held.
    pub fn plan_updates(
        &self,
        installed: &[InstalledExtension],
        now: Timestamp,
    ) -> Vec<PlannedAction> {
        let index = self.index(now).unwrap_or(&self.empty);
        plan_updates(installed, index, self.blocklist(), now)
    }

    /// Downloads a release's package and checks it against the signed size and SHA-256 before
    /// the install store unpacks it.
    pub async fn download(
        &self,
        fetch: &impl Fetch,
        release: &Release,
    ) -> Result<Vec<u8>, DownloadError> {
        let request = FetchRequest {
            file: RegistryFile::Package {
                url: release.package.url.clone(),
            },
            max_bytes: release.package.size,
        };
        let bytes = fetch_capped(fetch, &request)
            .await
            .map_err(DownloadError::Fetch)?;
        release
            .verify_package(&bytes)
            .map_err(DownloadError::Mismatch)?;
        Ok(bytes)
    }

    fn refuse(&mut self, error: Error) -> IndexRefresh {
        self.refused = Some(error.clone());
        IndexRefresh::Refused(error)
    }

    /// Releases and block lists are checked against the compiled-in keyset minus what the last
    /// accepted snapshot revoked.
    fn blocklist_keyset(&self) -> Result<Keyset, Error> {
        match &self.snapshot {
            Some(snapshot) => snapshot.effective_keyset(&self.keyset),
            None => Ok(self.keyset.clone()),
        }
    }

    /// A newly accepted snapshot may revoke the key the cached block list is signed with: such a
    /// cache is dropped, so a list from a revoked key never blocks or unblocks anything.
    fn recheck_blocklist(&mut self) {
        let Some(cache) = &self.blocklist else {
            return;
        };
        let still_trusted = self.blocklist_keyset().is_ok_and(|keyset| {
            VerifiedBlocklist::verify(cache.blocklist.signed().clone(), &keyset).is_ok()
        });
        if !still_trusted {
            self.blocklist = None;
        }
    }
}

async fn fetch_capped(fetch: &impl Fetch, request: &FetchRequest) -> anyhow::Result<Vec<u8>> {
    let bytes = fetch.fetch(request).await?;
    anyhow::ensure!(
        bytes.len() as u64 <= request.max_bytes,
        "{:?} sent {} bytes, more than the {} expected",
        request.file,
        bytes.len(),
        request.max_bytes
    );
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        pin::pin,
        sync::Mutex,
        task::{Context, Poll, Waker},
        time::Duration,
    };

    use super::*;
    use crate::{
        Action, InstalledVersion, Origin, Sha256Digest, Signed, Snapshot, Tier,
        blocklist::test_support::{blocklist, entry, signed_json},
        index::test_support::release,
        signing::test_keys::{keyset, registry_key, rotation_key},
    };

    const HOUR: u64 = 3_600;
    const PUBLISHED: u64 = 1_000_000;
    /// Past the 24 h age gate of releases published at `PUBLISHED`.
    const NOW: Timestamp = Timestamp(PUBLISHED + 30 * HOUR);

    /// The futures here never wait: the fetcher answers at once.
    fn ready<T>(future: impl Future<Output = T>) -> T {
        match pin!(future).poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("the test fetcher never waits"),
        }
    }

    #[derive(Default)]
    struct Files {
        files: Mutex<HashMap<String, Vec<u8>>>,
        requests: Mutex<Vec<FetchRequest>>,
    }

    impl Files {
        fn key(file: &RegistryFile) -> String {
            match file {
                RegistryFile::Snapshot => "snapshot".into(),
                RegistryFile::Index => "index".into(),
                RegistryFile::Blocklist => "blocklist".into(),
                RegistryFile::Package { url } => url.clone(),
            }
        }

        fn put(&self, file: RegistryFile, bytes: Vec<u8>) {
            self.files.lock().unwrap().insert(Self::key(&file), bytes);
        }

        fn remove(&self, file: RegistryFile) {
            self.files.lock().unwrap().remove(&Self::key(&file));
        }

        fn take_requests(&self) -> Vec<RegistryFile> {
            std::mem::take(&mut *self.requests.lock().unwrap())
                .into_iter()
                .map(|request| request.file)
                .collect()
        }
    }

    impl Fetch for Files {
        fn fetch(
            &self,
            request: &FetchRequest,
        ) -> impl Future<Output = anyhow::Result<Vec<u8>>> + Send {
            self.requests.lock().unwrap().push(request.clone());
            let body = self
                .files
                .lock()
                .unwrap()
                .get(&Self::key(&request.file))
                .cloned();
            async move { body.ok_or_else(|| anyhow::anyhow!("404")) }
        }
    }

    fn package(id: &str, version: &str) -> Vec<u8> {
        format!("{id}-{version} package").into_bytes()
    }

    fn todo(version: &str) -> Release {
        let mut release = release("todo", version);
        let bytes = package("todo", version);
        release.package.sha256 = Sha256Digest::of(&bytes);
        release.package.size = bytes.len() as u64;
        release.published_at = Timestamp(PUBLISHED);
        release
    }

    fn index_bytes(releases: Vec<Release>) -> Vec<u8> {
        let mut index = Index::new(Timestamp(PUBLISHED));
        index.releases = releases
            .into_iter()
            .map(|release| Signed::sign(release, &registry_key()))
            .collect();
        index.to_json().expect("serializes")
    }

    fn snapshot_bytes(
        index: &[u8],
        sequence: u64,
        signer: &crate::SigningKey,
        revoked: &[&str],
    ) -> Vec<u8> {
        let snapshot = Snapshot::for_index(
            index,
            sequence,
            Timestamp(PUBLISHED + 29 * HOUR),
            Duration::from_secs(48 * HOUR),
            revoked
                .iter()
                .map(|key| crate::KeyId::new(*key).expect("valid"))
                .collect(),
        );
        Snapshot::to_json(&Signed::sign(snapshot, signer)).expect("serializes")
    }

    /// A registry serving `releases` under snapshot `sequence`.
    fn publish(files: &Files, releases: Vec<Release>, sequence: u64) -> Vec<u8> {
        let index = index_bytes(releases);
        files.put(
            RegistryFile::Snapshot,
            snapshot_bytes(&index, sequence, &registry_key(), &[]),
        );
        files.put(RegistryFile::Index, index.clone());
        index
    }

    fn id() -> ExtensionId {
        ExtensionId::new("todo").expect("valid")
    }

    fn installed(version: &str) -> InstalledExtension {
        InstalledExtension {
            id: id(),
            active: InstalledVersion {
                version: version.parse().expect("valid"),
                origin: Origin::Registry,
                package_sha256: Some(Sha256Digest::of(&package("todo", version))),
            },
            previous: None,
            pinned: false,
            approved: Permissions::of(&todo(version)),
        }
    }

    #[test]
    fn verifies_the_snapshot_then_the_index_then_the_block_list() {
        let files = Files::default();
        publish(&files, vec![todo("1.0.0"), todo("1.1.0")], 1);
        files.put(
            RegistryFile::Blocklist,
            signed_json(blocklist(1, vec![entry("todo", "=1.1.0")]), &registry_key()),
        );
        let mut client = RegistryClient::new(keyset());
        assert_eq!(client.index(NOW).err(), Some(Unavailable::NeverFetched));

        assert_eq!(
            ready(client.refresh_index(&files, NOW)),
            IndexRefresh::Updated
        );
        assert_eq!(
            files.take_requests(),
            [RegistryFile::Snapshot, RegistryFile::Index]
        );
        assert!(matches!(
            ready(client.refresh_blocklist(&files, NOW)),
            CacheUpdate::Replace(_)
        ));

        // 1.1.0 is blocked, so a fresh install gets 1.0.0, with no consent for an all-T0 release.
        let plan = client.plan_install(&id(), NOW).expect("installable");
        assert_eq!(plan.release.version.to_string(), "1.0.0");
        assert!(plan.consent.is_empty());
        let bytes = ready(client.download(&files, plan.release));
        assert!(
            matches!(bytes, Err(DownloadError::Fetch(_))),
            "not published yet"
        );
        files.put(
            RegistryFile::Package {
                url: plan.release.package.url.clone(),
            },
            package("todo", "1.0.0"),
        );
        assert_eq!(
            ready(client.download(&files, plan.release)).expect("matches"),
            package("todo", "1.0.0")
        );

        // The same snapshot again fetches no index.
        files.take_requests();
        assert_eq!(
            ready(client.refresh_index(&files, NOW)),
            IndexRefresh::Unchanged
        );
        assert_eq!(files.take_requests(), [RegistryFile::Snapshot]);
    }

    #[test]
    fn a_tampered_package_is_refused() {
        let files = Files::default();
        publish(&files, vec![todo("1.0.0")], 1);
        let mut client = RegistryClient::new(keyset());
        ready(client.refresh_index(&files, NOW));
        let release = client
            .plan_install(&id(), NOW)
            .expect("installable")
            .release;
        let url = RegistryFile::Package {
            url: release.package.url.clone(),
        };
        let mut tampered = package("todo", "1.0.0");
        tampered[0] ^= 1;
        files.put(url.clone(), tampered);
        assert!(matches!(
            ready(client.download(&files, release)),
            Err(DownloadError::Mismatch(Error::PackageMismatch(_)))
        ));
        // One byte too many is refused before hashing.
        files.put(url, [package("todo", "1.0.0"), vec![0]].concat());
        assert!(matches!(
            ready(client.download(&files, release)),
            Err(DownloadError::Fetch(_))
        ));
    }

    #[test]
    fn a_rolled_back_or_expired_snapshot_stops_installs_and_updates() {
        let files = Files::default();
        let old_index = publish(&files, vec![todo("1.0.0")], 1);
        let old_snapshot = snapshot_bytes(&old_index, 1, &registry_key(), &[]);
        let mut client = RegistryClient::new(keyset());
        ready(client.refresh_index(&files, NOW));
        publish(&files, vec![todo("1.0.0"), todo("1.1.0")], 2);
        assert_eq!(
            ready(client.refresh_index(&files, NOW)),
            IndexRefresh::Updated
        );
        assert_eq!(client.plan_updates(&[installed("1.0.0")], NOW).len(), 1);

        // A mirror replays the first snapshot and index.
        files.put(RegistryFile::Snapshot, old_snapshot);
        files.put(RegistryFile::Index, old_index);
        assert!(matches!(
            ready(client.refresh_index(&files, NOW)),
            IndexRefresh::Refused(Error::Rollback { .. })
        ));
        assert!(matches!(
            client.plan_install(&id(), NOW),
            Err(InstallError::Unavailable(Unavailable::Refused(
                Error::Rollback { .. }
            )))
        ));
        assert!(client.plan_updates(&[installed("1.0.0")], NOW).is_empty());

        // A good snapshot clears it.
        publish(&files, vec![todo("1.0.0"), todo("1.1.0")], 3);
        assert_eq!(
            ready(client.refresh_index(&files, NOW)),
            IndexRefresh::Updated
        );
        assert!(client.plan_install(&id(), NOW).is_ok());

        // Offline is fine while the snapshot is valid; past its expiry, installs stop.
        files.remove(RegistryFile::Snapshot);
        assert!(matches!(
            ready(client.refresh_index(&files, NOW)),
            IndexRefresh::FetchFailed(_)
        ));
        assert!(client.plan_install(&id(), NOW).is_ok());
        let later = Timestamp(PUBLISHED + 29 * HOUR + 48 * HOUR);
        assert!(matches!(
            client.plan_install(&id(), later),
            Err(InstallError::Unavailable(Unavailable::Expired { .. }))
        ));
    }

    #[test]
    fn an_index_the_snapshot_does_not_name_is_refused() {
        let files = Files::default();
        let index = publish(&files, vec![todo("1.0.0")], 1);
        let mut client = RegistryClient::new(keyset());

        // A longer index is cut off at the size the snapshot names.
        files.put(
            RegistryFile::Index,
            index_bytes(vec![todo("1.0.0"), todo("9.0.0")]),
        );
        assert!(matches!(
            ready(client.refresh_index(&files, NOW)),
            IndexRefresh::FetchFailed(_)
        ));
        assert_eq!(
            files
                .requests
                .lock()
                .unwrap()
                .last()
                .map(|request| request.max_bytes),
            Some(index.len() as u64)
        );
        assert_eq!(client.index(NOW).err(), Some(Unavailable::NeverFetched));

        // One of the same size but other content fails the snapshot's hash.
        let mut tampered = index;
        let at = tampered
            .windows(4)
            .position(|window| window == b"Todo")
            .expect("the name is in the index");
        tampered[at] = b'X';
        files.put(RegistryFile::Index, tampered);
        assert!(matches!(
            ready(client.refresh_index(&files, NOW)),
            IndexRefresh::Refused(_)
        ));
        assert!(matches!(client.index(NOW), Err(Unavailable::Refused(_))));
    }

    #[test]
    fn blocked_versions_still_roll_back_while_updates_are_stopped() {
        let files = Files::default();
        publish(&files, vec![todo("1.0.0"), todo("1.1.0")], 1);
        files.put(
            RegistryFile::Blocklist,
            signed_json(blocklist(1, vec![entry("todo", "=1.1.0")]), &registry_key()),
        );
        let mut client = RegistryClient::new(keyset());
        ready(client.refresh_index(&files, NOW));
        ready(client.refresh_blocklist(&files, NOW));
        let mut extension = installed("1.1.0");
        extension.previous = installed("1.0.0").active.into();
        let later = Timestamp(PUBLISHED + 29 * HOUR + 48 * HOUR);
        assert!(client.index(later).is_err());
        assert!(matches!(
            &client.plan_updates(&[extension], later)[..],
            [PlannedAction {
                action: Action::Rollback { .. },
                ..
            }]
        ));
    }

    #[test]
    fn held_updates_ask_for_new_t1_and_t2_capabilities() {
        let files = Files::default();
        let mut newer = todo("1.1.0");
        newer.capabilities.insert("network:fetch".into());
        newer.tier = Tier::T2;
        newer.network_domains.insert("api.example.com".into());
        publish(&files, vec![todo("1.0.0"), newer.clone()], 1);
        let mut client = RegistryClient::new(keyset());
        ready(client.refresh_index(&files, NOW));
        let mut extension = installed("1.0.0");
        let plan = client.plan_updates(std::slice::from_ref(&extension), NOW);
        assert!(
            matches!(&plan[..], [PlannedAction { action: Action::Hold { reasons, .. }, .. }]
                if matches!(&reasons[..], [crate::HoldReason::NeedsApproval(additions)]
                    if additions.tier_raised == Some(Tier::T2)
                        && additions.network_domains.contains("api.example.com"))),
            "{plan:?}"
        );
        extension.approved.approve(&newer);
        assert!(matches!(
            &client.plan_updates(&[extension], NOW)[..],
            [PlannedAction {
                action: Action::Update { .. },
                ..
            }]
        ));
    }

    #[test]
    fn restores_what_it_stored() {
        let files = Files::default();
        publish(&files, vec![todo("1.0.0")], 4);
        files.put(
            RegistryFile::Blocklist,
            signed_json(blocklist(1, vec![entry("todo", "=0.9.0")]), &registry_key()),
        );
        let mut client = RegistryClient::new(keyset());
        ready(client.refresh_index(&files, NOW));
        ready(client.refresh_blocklist(&files, NOW));
        let stored = client.stored_state().expect("serializes");

        let (restored, problems) = RegistryClient::restore(keyset(), &stored);
        assert!(problems.is_empty(), "{problems:?}");
        assert!(restored.plan_install(&id(), NOW).is_ok());
        assert_eq!(
            restored.blocklist().map(VerifiedBlocklist::sequence),
            Some(1)
        );
        assert_eq!(restored.stored_state().expect("serializes"), stored);

        // A damaged index is left out and fetched again; the snapshot still guards the sequence.
        let damaged = StoredState {
            index: Some(b"{}".to_vec()),
            ..stored
        };
        let (mut restored, problems) = RegistryClient::restore(keyset(), &damaged);
        assert_eq!(problems.len(), 1);
        assert_eq!(restored.index(NOW).err(), Some(Unavailable::NeverFetched));
        assert_eq!(
            ready(restored.refresh_index(&files, NOW)),
            IndexRefresh::Updated
        );
        publish(&files, vec![todo("1.0.0")], 3);
        assert!(matches!(
            ready(restored.refresh_index(&files, NOW)),
            IndexRefresh::Refused(Error::Rollback { .. })
        ));
    }

    /// A rotation-key snapshot that revokes the block-list key drops the list it signed, and
    /// lists signed by that key are refused from then on.
    #[test]
    fn a_revoked_block_list_key_stops_counting() {
        let files = Files::default();
        let index = publish(&files, vec![todo("1.0.0")], 1);
        files.put(
            RegistryFile::Blocklist,
            signed_json(blocklist(1, vec![entry("todo", "*")]), &registry_key()),
        );
        let mut client = RegistryClient::new(keyset());
        ready(client.refresh_index(&files, NOW));
        ready(client.refresh_blocklist(&files, NOW));
        assert!(client.blocklist().is_some());

        files.put(
            RegistryFile::Snapshot,
            snapshot_bytes(&index, 2, &rotation_key(), &["test-registry"]),
        );
        // The releases are signed by the revoked key too, so none is left to install.
        assert_eq!(
            ready(client.refresh_index(&files, NOW)),
            IndexRefresh::Updated
        );
        assert!(client.blocklist().is_none());
        assert!(matches!(
            ready(client.refresh_blocklist(&files, NOW)),
            CacheUpdate::Keep(crate::KeepReason::Invalid(_))
        ));
        assert!(matches!(
            client.plan_install(&id(), NOW),
            Err(InstallError::Refused(InstallRefusal::NotInIndex))
        ));
    }
}
