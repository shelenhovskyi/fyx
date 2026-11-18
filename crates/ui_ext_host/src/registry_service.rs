//! The Fyx extension registry on the host side (platform plan 3.3–3.5): where it is, how its
//! files are fetched, what is trusted, and what is kept between runs. The decisions are
//! `fyx_registry::RegistryClient`'s; this module does the I/O around it.
//!
//! - **Where:** `fyx.extensions.registry_url`, empty (off) by default. An `https://` base URL;
//!   `http://` only on a loopback host (a local test server); or `file://` for a directory (the
//!   local test registry). A directory registry maps a release's `https://` package URL to the
//!   file at the same path under it.
//! - **Trust:** the compiled-in keyset. Debug and test builds trust the test keys in
//!   `fixtures/registry/test-keyset.json` (public keys only); release builds trust
//!   [`RELEASE_KEYSET_JSON`], empty until the registry publishes its keyset, so installs from
//!   the index stay stopped there.
//! - **When:** nothing is fetched at startup. The Fyx tab refreshes when it opens, and an update
//!   check runs every [`UPDATE_CHECK_INTERVAL`]; the block list is fetched when it is due
//!   (every 6 h).
//! - **Kept:** the client's three state files under `<data dir>/extensions/fyx-registry/`,
//!   written after an accepted index or block list.

use std::{
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context as _, Result, bail};
use futures::{AsyncReadExt as _, lock::Mutex};
use fyx_registry::{
    Action, CacheUpdate, ExtensionId, Fetch, FetchRequest, HoldReason, IndexRefresh,
    InstalledExtension, Keyset, RegistryClient, RegistryFile, Release, StoredState, Timestamp,
};
use http_client::{AsyncBody, HttpClient};

/// The keys release builds trust: none yet. When the registry repository publishes its keyset,
/// its JSON goes here (registry-format.md, "Keys").
pub const RELEASE_KEYSET_JSON: Option<&str> = None;

/// The test keyset: public keys only. The matching private seeds exist only in tests and the
/// local test-registry tooling.
#[cfg(any(debug_assertions, test, feature = "test-support"))]
pub const TEST_KEYSET_JSON: &str = include_str!("../fixtures/registry/test-keyset.json");

/// How often installed registry extensions are checked for updates and blocks.
pub const UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// The keyset this build trusts, or `None` when it trusts none (installs from the index stop).
pub fn compiled_keyset() -> Option<Keyset> {
    #[cfg(any(debug_assertions, test, feature = "test-support"))]
    {
        return Keyset::from_json(TEST_KEYSET_JSON.as_bytes()).ok();
    }
    #[allow(unreachable_code)]
    release_keyset()
}

/// The keyset of a release build.
pub fn release_keyset() -> Option<Keyset> {
    RELEASE_KEYSET_JSON.and_then(|json| match Keyset::from_json(json.as_bytes()) {
        Ok(keyset) => Some(keyset),
        Err(error) => {
            log::error!("fyx:ui: the compiled-in registry keyset is invalid: {error}");
            None
        }
    })
}

pub fn now() -> Timestamp {
    Timestamp(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs()),
    )
}

/// Where the registry's files are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistryLocation {
    /// A base URL ending in `/`.
    Http(String),
    /// A directory: `snapshot.json`, `index.json`, `blocklist.json`, and the packages at the
    /// paths of their URLs.
    Directory(PathBuf),
}

impl RegistryLocation {
    /// Parses `fyx.extensions.registry_url`. `Ok(None)` when it is empty (the registry is off).
    pub fn parse(url: &str) -> Result<Option<Self>> {
        let url = url.trim();
        if url.is_empty() {
            return Ok(None);
        }
        if let Some(path) = url.strip_prefix("file://") {
            // `file:///C:/x` on Windows, `file:///x` elsewhere.
            let path = if cfg!(windows) {
                path.trim_start_matches('/')
            } else {
                path
            };
            return Ok(Some(Self::Directory(PathBuf::from(path))));
        }
        let base = if url.ends_with('/') {
            url.to_string()
        } else {
            format!("{url}/")
        };
        if let Some(rest) = base.strip_prefix("https://") {
            if rest.starts_with('/') || rest.len() <= 1 {
                bail!("{url:?} has no host");
            }
            return Ok(Some(Self::Http(base)));
        }
        if let Some(rest) = base.strip_prefix("http://") {
            let host = rest.split(['/', ':']).next().unwrap_or_default();
            let authority = rest.split('/').next().unwrap_or_default();
            let loopback =
                matches!(host, "localhost" | "127.0.0.1") || authority.starts_with("[::1]");
            if loopback {
                return Ok(Some(Self::Http(base)));
            }
            bail!("{url:?}: a registry needs https://, plain http:// only on this computer");
        }
        bail!("{url:?} is not an https://, http://localhost or file:// URL")
    }
}

enum Target {
    Url(String),
    File(PathBuf),
}

/// HTTP (or directory) access to the registry's files, for `RegistryClient`.
#[derive(Clone)]
pub struct RegistryFetch {
    location: RegistryLocation,
    http: Arc<dyn HttpClient>,
}

impl RegistryFetch {
    pub fn new(location: RegistryLocation, http: Arc<dyn HttpClient>) -> Self {
        Self { location, http }
    }

    fn target(&self, file: &RegistryFile) -> Result<Target> {
        let name = match file {
            RegistryFile::Snapshot => "snapshot.json",
            RegistryFile::Index => "index.json",
            RegistryFile::Blocklist => "blocklist.json",
            RegistryFile::Package { url } => return self.package_target(url),
        };
        Ok(match &self.location {
            RegistryLocation::Http(base) => Target::Url(format!("{base}{name}")),
            RegistryLocation::Directory(dir) => Target::File(dir.join(name)),
        })
    }

    /// A signed release names an `https://` URL. Over HTTP it is fetched as is; a directory
    /// registry serves it from the same path under the directory.
    fn package_target(&self, url: &str) -> Result<Target> {
        let Some(rest) = url.strip_prefix("https://") else {
            bail!("package URL {url:?} is not https://");
        };
        match &self.location {
            RegistryLocation::Http(_) => Ok(Target::Url(url.to_string())),
            RegistryLocation::Directory(dir) => {
                let path = rest
                    .split_once('/')
                    .map(|(_, path)| path)
                    .unwrap_or_default();
                let relative = Path::new(path);
                let plain = !path.is_empty()
                    && relative
                        .components()
                        .all(|component| matches!(component, Component::Normal(_)));
                if !plain {
                    bail!("package URL {url:?} has no plain path for a directory registry");
                }
                Ok(Target::File(dir.join(relative)))
            }
        }
    }
}

impl Fetch for RegistryFetch {
    fn fetch(
        &self,
        request: &FetchRequest,
    ) -> impl std::future::Future<Output = Result<Vec<u8>>> + Send {
        let target = self.target(&request.file);
        let http = self.http.clone();
        let max_bytes = request.max_bytes;
        async move {
            match target? {
                Target::File(path) => {
                    let size = std::fs::metadata(&path)
                        .with_context(|| format!("reading {}", path.display()))?
                        .len();
                    if size > max_bytes {
                        bail!("{} is {size} bytes, more than {max_bytes}", path.display());
                    }
                    std::fs::read(&path).with_context(|| format!("reading {}", path.display()))
                }
                Target::Url(url) => {
                    let mut response = http.get(&url, AsyncBody::empty(), true).await?;
                    let status = response.status();
                    if !status.is_success() {
                        bail!("{url}: HTTP {status}");
                    }
                    // Stops reading past the cap instead of taking whatever the server sends.
                    let mut body = Vec::new();
                    response
                        .body_mut()
                        .take(max_bytes.saturating_add(1))
                        .read_to_end(&mut body)
                        .await
                        .with_context(|| format!("reading {url}"))?;
                    if body.len() as u64 > max_bytes {
                        bail!("{url} is more than {max_bytes} bytes");
                    }
                    Ok(body)
                }
            }
        }
    }
}

/// Where the client's state files live.
#[derive(Clone, Debug)]
pub struct RegistryStateDir(pub PathBuf);

impl RegistryStateDir {
    pub fn default_dir() -> Self {
        Self(paths::data_dir().join("extensions").join("fyx-registry"))
    }

    const SNAPSHOT: &str = "snapshot-state.json";
    const LAST_CHECK: &str = "last-check";
    const INDEX: &str = "index.json";
    const BLOCKLIST: &str = "blocklist.json";

    /// Reads what is there; a missing file is a missing part.
    pub fn load(&self) -> StoredState {
        let read = |name: &str| std::fs::read(self.0.join(name)).ok();
        StoredState {
            snapshot: read(Self::SNAPSHOT),
            index: read(Self::INDEX),
            blocklist: read(Self::BLOCKLIST),
        }
    }

    /// When the last registry check started (decision 3: the 6 h timer counts from it, across
    /// restarts).
    pub fn last_check(&self) -> Option<Timestamp> {
        let text = std::fs::read_to_string(self.0.join(Self::LAST_CHECK)).ok()?;
        text.trim().parse().ok().map(Timestamp)
    }

    pub fn set_last_check(&self, at: Timestamp) -> Result<()> {
        std::fs::create_dir_all(&self.0)
            .with_context(|| format!("creating {}", self.0.display()))?;
        let path = self.0.join(Self::LAST_CHECK);
        std::fs::write(&path, at.0.to_string())
            .with_context(|| format!("writing {}", path.display()))
    }

    pub fn store(&self, state: &StoredState) -> Result<()> {
        std::fs::create_dir_all(&self.0)
            .with_context(|| format!("creating {}", self.0.display()))?;
        for (name, part) in [
            (Self::SNAPSHOT, &state.snapshot),
            (Self::INDEX, &state.index),
            (Self::BLOCKLIST, &state.blocklist),
        ] {
            let path = self.0.join(name);
            match part {
                Some(bytes) => {
                    let partial = path.with_extension("json.partial");
                    std::fs::write(&partial, bytes)
                        .and_then(|()| std::fs::rename(&partial, &path))
                        .with_context(|| format!("writing {}", path.display()))?;
                }
                None => {
                    std::fs::remove_file(&path).ok();
                }
            }
        }
        Ok(())
    }
}

/// What a refresh did, for the page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefreshOutcome {
    pub index: IndexRefresh,
    /// `None` when the block list was not due.
    pub blocklist: Option<CacheUpdate>,
}

/// The client with its fetcher and state files. All methods run off the main thread; the
/// client sits behind an async lock so a refresh and an install never interleave.
#[derive(Clone)]
pub struct Registry {
    client: Arc<Mutex<RegistryClient>>,
    fetch: RegistryFetch,
    state_dir: RegistryStateDir,
    /// The last block-list fetch attempt, good or not, so a failing mirror is not asked on
    /// every refresh.
    last_blocklist_attempt: Arc<std::sync::Mutex<Option<Timestamp>>>,
}

impl Registry {
    /// Restores the stored state (no network). `None` when this build trusts no keys.
    pub fn open(
        keyset: Option<Keyset>,
        location: RegistryLocation,
        http: Arc<dyn HttpClient>,
        state_dir: RegistryStateDir,
    ) -> Option<Self> {
        let keyset = keyset?;
        let (client, problems) = RegistryClient::restore(keyset, &state_dir.load());
        for problem in problems {
            log::warn!("fyx:ui: registry state not restored: {problem}");
        }
        let last_blocklist_attempt = client.blocklist_cache().map(|cache| cache.fetched_at);
        Some(Self {
            client: Arc::new(Mutex::new(client)),
            fetch: RegistryFetch::new(location, http),
            state_dir,
            last_blocklist_attempt: Arc::new(std::sync::Mutex::new(last_blocklist_attempt)),
        })
    }

    pub fn client(&self) -> &Arc<Mutex<RegistryClient>> {
        &self.client
    }

    pub fn fetch(&self) -> &RegistryFetch {
        &self.fetch
    }

    /// The index, then the block list when it is due; the state is stored after anything new.
    pub async fn refresh(&self, now: Timestamp) -> RefreshOutcome {
        self.refresh_with(now, false).await
    }

    /// [`Self::refresh`]; with `blocklist_now` the block list is fetched even when not due (the
    /// "check for extension updates" command).
    pub async fn refresh_with(&self, now: Timestamp, blocklist_now: bool) -> RefreshOutcome {
        let mut client = self.client.lock().await;
        let index = client.refresh_index(&self.fetch, now).await;
        let last_attempt = *self
            .last_blocklist_attempt
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let blocklist = if blocklist_now
            || fyx_registry::BlocklistCache::fetch_due(last_attempt, now)
        {
            *self
                .last_blocklist_attempt
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(now);
            Some(client.refresh_blocklist(&self.fetch, now).await)
        } else {
            None
        };
        let changed = matches!(index, IndexRefresh::Updated)
            || matches!(
                blocklist,
                Some(CacheUpdate::Replace(_) | CacheUpdate::Refresh(_))
            );
        if changed {
            match client.stored_state() {
                Ok(state) => {
                    if let Err(error) = self.state_dir.store(&state) {
                        log::error!("fyx:ui: cannot store the registry state: {error:#}");
                    }
                }
                Err(error) => log::error!("fyx:ui: cannot store the registry state: {error}"),
            }
        }
        RefreshOutcome { index, blocklist }
    }

    /// Downloads and verifies the newest installable release of `id` (the age gate and the block
    /// list applied). Returns it with the package bytes.
    pub async fn download_for_install(
        &self,
        id: &ExtensionId,
        now: Timestamp,
    ) -> Result<(Release, Vec<u8>)> {
        let client = self.client.lock().await;
        let release = client
            .plan_install(id, now)
            .map_err(|error| anyhow::anyhow!("{id} cannot be installed: {error:?}"))?
            .release
            .clone();
        let bytes = client
            .download(&self.fetch, &release)
            .await
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        Ok((release, bytes))
    }

    /// Downloads and verifies an update of `installed` to `release`, if the plan, made again now,
    /// still names it: an update, or a held update that waits only for the user's approval
    /// (which the caller just got). A block or a pin that came since stops it.
    pub async fn download_planned_update(
        &self,
        installed: &InstalledExtension,
        release: &Release,
        now: Timestamp,
    ) -> Result<Vec<u8>> {
        let client = self.client.lock().await;
        let planned = client
            .plan_updates(std::slice::from_ref(installed), now)
            .into_iter()
            .any(|action| match action.action {
                Action::Update { to, .. } => to == release.version,
                Action::Hold { version, reasons } => {
                    version == release.version
                        && reasons
                            .iter()
                            .all(|reason| matches!(reason, HoldReason::NeedsApproval(_)))
                }
                _ => false,
            });
        anyhow::ensure!(
            planned,
            "{} {} is no longer an update the registry allows; refresh and try again",
            release.id,
            release.version
        );
        client
            .download(&self.fetch, release)
            .await
            .map_err(|error| anyhow::anyhow!("{error}"))
    }

    /// Downloads and verifies one release the caller's plan named: the update check's updates,
    /// planned in the same refresh. Every other install goes through
    /// [`Self::download_for_install`] or [`Self::download_planned_update`].
    pub(crate) async fn download(&self, release: &Release) -> Result<Vec<u8>> {
        let client = self.client.lock().await;
        client
            .download(&self.fetch, release)
            .await
            .map_err(|error| anyhow::anyhow!("{error}"))
    }
}
