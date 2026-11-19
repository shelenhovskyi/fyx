//! The install store (plan 1.1, platform spec decisions 12, 13 and 17): UI extensions installed
//! from `.fyxext` packages.
//!
//! Layout, under `<data dir>/extensions/fyx/`:
//! - `<id>/<version>/`: one installed version, unpacked (`extension.toml`, the component, assets);
//! - `<id>/<version>-<key>.cwasm`: that version's precompiled component, written by Fyx on first
//!   load (`ui_ext_wasm::PrecompiledCache`). It sits beside the version folder, never inside it,
//!   so nothing from a package can pose as one;
//! - `state.json`: per id, the active version, the previous one (kept for rollback), whether
//!   the id is pinned, where each kept version came from (the registry or a file) with its
//!   package's SHA-256, and the permissions the user approved (platform plan 3.3, 3.5).
//!
//! An install keeps the version it replaces as `previous` and deletes every other version. A
//! pinned id takes no other version until it is unpinned; rollback swaps `active` and `previous`.
//!
//! A package is a zip of `extension.toml`, the component it names, and optional `assets/` and
//! `signature.json`. It is refused when it is over [`MAX_PACKAGE_BYTES`], when its component is
//! over [`MAX_COMPONENT_BYTES`], when it unpacks to more than [`MAX_UNPACKED_BYTES`] or
//! [`MAX_PACKAGE_ENTRIES`] entries, or when an entry's name is not a plain relative path.
//! Signatures are not checked yet (that is the registry's, plan 3); every package installed from
//! disk is unsigned.

use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context as _, Result, bail};
use fs::{Fs, RemoveOptions, RenameOptions};
use futures::{AsyncReadExt as _, lock::Mutex};
use fyx_registry::{
    ExtensionId, InstalledExtension, InstalledVersion, Origin, Permissions, Release, Sha256Digest,
};
use semver::Version;
use serde::{Deserialize, Serialize};
use ui_ext_wasm::{PrecompiledCache, UiManifest};

pub const MAX_PACKAGE_BYTES: u64 = 50 << 20;
pub const MAX_COMPONENT_BYTES: u64 = 32 << 20;
pub const MAX_UNPACKED_BYTES: u64 = 128 << 20;
pub const MAX_PACKAGE_ENTRIES: usize = 1024;

const STATE_FILE: &str = "state.json";
const STAGING_DIR: &str = ".staging";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreState {
    pub extensions: BTreeMap<String, ExtensionState>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionState {
    pub active: Version,
    /// Kept for rollback.
    pub previous: Option<Version>,
    /// No other version installs while pinned.
    #[serde(default)]
    pub pinned: bool,
    /// Where each kept version came from, by version. A version without an entry (installed
    /// before this was recorded) counts as a file install with no known hash.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sources: BTreeMap<String, VersionSource>,
    /// What the user approved: a registry release's permissions when it was installed, plus
    /// those of held updates they approved (`Permissions::approve`).
    #[serde(default)]
    pub approved: Permissions,
    /// Why the active version is not loaded: a block-list entry with nothing to roll back to.
    /// Cleared when the block lifts or another version installs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled: Option<String>,
}

/// Where one installed version came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionSource {
    pub origin: Origin,
    pub package_sha256: Option<Sha256Digest>,
}

impl VersionSource {
    pub fn local_file(package: &[u8]) -> Self {
        Self {
            origin: Origin::LocalFile,
            package_sha256: Some(Sha256Digest::of(package)),
        }
    }
}

impl ExtensionState {
    fn installed_version(&self, version: &Version) -> InstalledVersion {
        let source = self.sources.get(&version.to_string());
        InstalledVersion {
            version: version.clone(),
            origin: source.map_or(Origin::LocalFile, |source| source.origin),
            package_sha256: source.and_then(|source| source.package_sha256),
        }
    }
}

/// What a package holds, read without installing it (for the install confirmation).
#[derive(Clone, Debug)]
pub struct PackageInfo {
    pub extension_id: Arc<str>,
    pub version: Version,
    pub manifest: UiManifest,
    /// The package also holds a Zed extension (`extension.wasm`).
    pub zed_extension: bool,
}

/// The result of an install.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    pub extension_id: Arc<str>,
    pub version: Version,
    pub previous: Option<Version>,
    pub dir: PathBuf,
}

#[derive(Clone)]
pub struct InstallStore {
    root: PathBuf,
    /// Serializes changes to the layout and `state.json`.
    lock: Arc<Mutex<()>>,
}

#[derive(Deserialize)]
struct VersionToml {
    version: String,
}

impl InstallStore {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            lock: Arc::new(Mutex::new(())),
        }
    }

    /// `<data dir>/extensions/fyx`.
    pub fn default_root() -> PathBuf {
        paths::data_dir().join("extensions").join("fyx")
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn version_dir(&self, extension_id: &str, version: &Version) -> PathBuf {
        self.root.join(extension_id).join(version.to_string())
    }

    /// Where this version's precompiled component lives; see [`PrecompiledCache`].
    pub fn precompiled_cache(&self, extension_id: &str, version: &Version) -> PrecompiledCache {
        PrecompiledCache {
            dir: self.root.join(extension_id),
            stem: version.to_string(),
        }
    }

    /// `state.json`, or an empty state when there is none yet.
    pub async fn state(&self, fs: &dyn Fs) -> Result<StoreState> {
        let path = self.root.join(STATE_FILE);
        if fs.metadata(&path).await?.is_none() {
            return Ok(StoreState::default());
        }
        let text = fs.load(&path).await?;
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    async fn write_state(&self, state: &StoreState, fs: &dyn Fs) -> Result<()> {
        fs.create_dir(&self.root).await?;
        fs.atomic_write(
            self.root.join(STATE_FILE),
            serde_json::to_string_pretty(state)?,
        )
        .await
    }

    /// The id, version and `[fyx_ui]` table of a package, after the same checks an install makes,
    /// without installing it.
    pub async fn inspect_package(&self, package: &Path, fs: &dyn Fs) -> Result<PackageInfo> {
        let staging = self.unpack_to_staging(package, fs).await?;
        let info = read_package_dir(&staging, fs).await;
        remove_dir(&staging, fs).await;
        info
    }

    /// A package file's bytes, read once, so what the install dialog showed is what installs
    /// (integration-5 R5).
    pub async fn read_package_file(&self, package: &Path, fs: &dyn Fs) -> Result<Vec<u8>> {
        self.load_package(package, fs).await
    }

    /// [`Self::inspect_package`] of bytes already read.
    pub async fn inspect_package_bytes(
        &self,
        bytes: Vec<u8>,
        label: &str,
        fs: &dyn Fs,
    ) -> Result<PackageInfo> {
        // Any `extension.wasm` in the archive, at any depth or case (owner's Q1).
        let zed_extension = fyx_registry::runs_zed_extension(&bytes);
        let staging = self.unpack_bytes_to_staging(bytes, label, fs).await?;
        let info = read_package_dir(&staging, fs).await;
        remove_dir(&staging, fs).await;
        info.map(|mut info| {
            info.zed_extension |= zed_extension;
            info
        })
    }

    /// Installs a verified registry download of `release`. The package must be that release
    /// (integration-5 R7): the same id and version, and no capability, domain or program beyond
    /// what the index entry, which the install dialog showed, declares.
    pub async fn install_registry_package(
        &self,
        bytes: Vec<u8>,
        release: &Release,
        approved: Option<Permissions>,
        fs: &dyn Fs,
    ) -> Result<Installed> {
        let label = format!("{} {}", release.id, release.version);
        let staging = self.unpack_bytes_to_staging(bytes, &label, fs).await?;
        let result = async {
            let info = read_package_dir(&staging, fs).await?;
            check_matches_release(&info, release)?;
            let source = VersionSource {
                origin: Origin::Registry,
                package_sha256: Some(release.package.sha256),
            };
            self.install_staged(&staging, source, approved, fs).await
        }
        .await;
        remove_dir(&staging, fs).await;
        result.with_context(|| format!("installing {label}"))
    }

    /// Installs a `.fyxext` package from disk (unsigned) and makes it the active version of its
    /// id. The version it replaces becomes `previous`; every other version of the id is deleted.
    pub async fn install_package(&self, package: &Path, fs: &dyn Fs) -> Result<Installed> {
        let bytes = self.load_package(package, fs).await?;
        let source = VersionSource::local_file(&bytes);
        self.install_package_bytes(bytes, &package.display().to_string(), source, None, fs)
            .await
    }

    /// Installs a package already in memory (a verified registry download): `source` says where
    /// it came from, `approved` what the user accepted for it (`None` keeps what was approved).
    pub async fn install_package_bytes(
        &self,
        bytes: Vec<u8>,
        label: &str,
        source: VersionSource,
        approved: Option<Permissions>,
        fs: &dyn Fs,
    ) -> Result<Installed> {
        let staging = self.unpack_bytes_to_staging(bytes, label, fs).await?;
        let result = self.install_staged(&staging, source, approved, fs).await;
        remove_dir(&staging, fs).await;
        result.with_context(|| format!("installing {label}"))
    }

    /// Disables the active version of an id (`Some(reason)`), or enables it again.
    pub async fn set_disabled(
        &self,
        extension_id: &str,
        reason: Option<String>,
        fs: &dyn Fs,
    ) -> Result<()> {
        let _guard = self.lock.lock().await;
        let mut state = self.state(fs).await?;
        let Some(entry) = state.extensions.get_mut(extension_id) else {
            bail!("{extension_id} is not installed");
        };
        entry.disabled = reason;
        self.write_state(&state, fs).await
    }

    /// Records approved permissions for an installed id (a held update the user approved).
    pub async fn set_approved(
        &self,
        extension_id: &str,
        approved: Permissions,
        fs: &dyn Fs,
    ) -> Result<()> {
        let _guard = self.lock.lock().await;
        let mut state = self.state(fs).await?;
        let Some(entry) = state.extensions.get_mut(extension_id) else {
            bail!("{extension_id} is not installed");
        };
        entry.approved = approved;
        self.write_state(&state, fs).await
    }

    /// Takes one capability out of an installed id's approval, read and written under the store
    /// lock so a concurrent install or approval is not overwritten. `false` when the id is not
    /// installed or its approval did not have the capability.
    pub async fn take_back_approval(
        &self,
        extension_id: &str,
        capability: &str,
        fs: &dyn Fs,
    ) -> Result<bool> {
        let _guard = self.lock.lock().await;
        let mut state = self.state(fs).await?;
        let Some(entry) = state.extensions.get_mut(extension_id) else {
            return Ok(false);
        };
        if !entry.approved.capabilities.remove(capability) {
            return Ok(false);
        }
        self.write_state(&state, fs).await?;
        Ok(true)
    }

    /// Every installed id as the registry's update planning sees it (`plan_updates`). An id the
    /// registry would not accept is left out.
    pub async fn installed_extensions(&self, fs: &dyn Fs) -> Result<Vec<InstalledExtension>> {
        let state = self.state(fs).await?;
        Ok(state
            .extensions
            .iter()
            .filter_map(|(id, entry)| {
                let id = ExtensionId::new(id.clone()).ok()?;
                Some(InstalledExtension {
                    id,
                    active: entry.installed_version(&entry.active),
                    previous: entry
                        .previous
                        .as_ref()
                        .map(|previous| entry.installed_version(previous)),
                    pinned: entry.pinned,
                    approved: entry.approved.clone(),
                })
            })
            .collect())
    }

    async fn install_staged(
        &self,
        staging: &Path,
        source: VersionSource,
        approved: Option<Permissions>,
        fs: &dyn Fs,
    ) -> Result<Installed> {
        let info = read_package_dir(staging, fs).await?;
        let _guard = self.lock.lock().await;
        let mut state = self.state(fs).await?;
        let id = info.extension_id.to_string();
        let current = state.extensions.get(&id).cloned();
        if let Some(current) = &current
            && current.pinned
            && current.active != info.version
        {
            bail!(
                "{id} is pinned to {}; unpin it to install {}",
                current.active,
                info.version
            );
        }
        let dir = self.version_dir(&id, &info.version);
        if fs.metadata(&dir).await?.is_some() {
            remove_dir_checked(&dir, fs).await?;
        }
        fs.rename(
            staging,
            &dir,
            RenameOptions {
                overwrite: false,
                ignore_if_exists: false,
                create_parents: true,
            },
        )
        .await
        .with_context(|| format!("moving the package into {}", dir.display()))?;

        let previous = match &current {
            Some(current) if current.active != info.version => Some(current.active.clone()),
            Some(current) => current.previous.clone(),
            None => None,
        };
        let mut sources = current
            .as_ref()
            .map(|current| current.sources.clone())
            .unwrap_or_default();
        sources.insert(info.version.to_string(), source);
        let kept = [Some(&info.version), previous.as_ref()];
        sources.retain(|version, _| {
            kept.iter()
                .flatten()
                .any(|kept| kept.to_string() == *version)
        });
        let approved = approved
            .or_else(|| current.as_ref().map(|current| current.approved.clone()))
            .unwrap_or_default();
        state.extensions.insert(
            id.clone(),
            ExtensionState {
                active: info.version.clone(),
                previous: previous.clone(),
                pinned: current.is_some_and(|current| current.pinned),
                sources,
                approved,
                disabled: None,
            },
        );
        self.write_state(&state, fs).await?;
        self.delete_other_versions(&id, &state.extensions[&id], fs)
            .await;
        Ok(Installed {
            extension_id: info.extension_id,
            version: info.version,
            previous,
            dir,
        })
    }

    /// Makes `previous` active again and the active version `previous`, so a second rollback
    /// undoes the first. Returns the new active version.
    pub async fn rollback(&self, extension_id: &str, fs: &dyn Fs) -> Result<Version> {
        let _guard = self.lock.lock().await;
        let mut state = self.state(fs).await?;
        let Some(entry) = state.extensions.get_mut(extension_id) else {
            bail!("{extension_id} is not installed");
        };
        let Some(previous) = entry.previous.take() else {
            bail!("{extension_id} has no previous version to roll back to");
        };
        let dir = self.version_dir(extension_id, &previous);
        if fs.metadata(&dir).await?.is_none() {
            bail!(
                "{extension_id} {previous} is missing from {}",
                dir.display()
            );
        }
        entry.previous = Some(std::mem::replace(&mut entry.active, previous.clone()));
        // The reason was for the version that is no longer active; the next check disables
        // this one again if it is blocked too (integration-5 R9).
        entry.disabled = None;
        self.write_state(&state, fs).await?;
        Ok(previous)
    }

    pub async fn set_pinned(&self, extension_id: &str, pinned: bool, fs: &dyn Fs) -> Result<()> {
        let _guard = self.lock.lock().await;
        let mut state = self.state(fs).await?;
        let Some(entry) = state.extensions.get_mut(extension_id) else {
            bail!("{extension_id} is not installed");
        };
        entry.pinned = pinned;
        self.write_state(&state, fs).await
    }

    /// Removes every version of the id and its entry. The id must be a valid extension id, so a
    /// tampered `state.json` cannot name a folder outside the store (integration-5 R9).
    pub async fn uninstall(&self, extension_id: &str, fs: &dyn Fs) -> Result<()> {
        if ExtensionId::new(extension_id).is_err() {
            bail!("{extension_id:?} is not an extension id");
        }
        let _guard = self.lock.lock().await;
        let mut state = self.state(fs).await?;
        if state.extensions.remove(extension_id).is_none() {
            bail!("{extension_id} is not installed");
        }
        self.write_state(&state, fs).await?;
        remove_dir_checked(&self.root.join(extension_id), fs).await
    }

    /// The active version folder of every installed id that is not disabled.
    pub async fn active(&self, fs: &dyn Fs) -> Result<Vec<(Arc<str>, Version, PathBuf)>> {
        let state = self.state(fs).await?;
        Ok(state
            .extensions
            .into_iter()
            .filter(|(_, entry)| entry.disabled.is_none())
            .map(|(id, entry)| {
                let dir = self.version_dir(&id, &entry.active);
                (id.into(), entry.active, dir)
            })
            .collect())
    }

    /// Deletes the versions (and precompiled components) of `id` other than `active` and
    /// `previous`. Failures are logged: a leftover folder costs disk space, nothing else.
    async fn delete_other_versions(&self, id: &str, entry: &ExtensionState, fs: &dyn Fs) {
        let keep: Vec<String> = std::iter::once(&entry.active)
            .chain(entry.previous.as_ref())
            .map(Version::to_string)
            .collect();
        let dir = self.root.join(id);
        let Ok(mut entries) = fs.read_dir(&dir).await else {
            return;
        };
        use futures::StreamExt as _;
        while let Some(Ok(path)) = entries.next().await {
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let kept = keep.iter().any(|version| {
                name == version
                    || name
                        .strip_prefix(version.as_str())
                        .is_some_and(|rest| rest.starts_with('-') && rest.ends_with(".cwasm"))
            });
            if kept {
                continue;
            }
            let result = if fs.is_dir(&path).await {
                fs.remove_dir(
                    &path,
                    RemoveOptions {
                        recursive: true,
                        ignore_if_not_exists: true,
                    },
                )
                .await
            } else {
                fs.remove_file(
                    &path,
                    RemoveOptions {
                        recursive: false,
                        ignore_if_not_exists: true,
                    },
                )
                .await
            };
            if let Err(error) = result {
                log::warn!("fyx:ui: could not delete {}: {error:#}", path.display());
            }
        }
    }

    /// Checks and unpacks a package into a fresh staging folder, which the caller removes.
    async fn load_package(&self, package: &Path, fs: &dyn Fs) -> Result<Vec<u8>> {
        let size = fs
            .metadata(package)
            .await?
            .with_context(|| format!("{} does not exist", package.display()))?
            .len;
        if size > MAX_PACKAGE_BYTES {
            bail!(
                "{} is {size} bytes; a package may be at most {MAX_PACKAGE_BYTES}",
                package.display()
            );
        }
        fs.load_bytes(package).await
    }

    async fn unpack_to_staging(&self, package: &Path, fs: &dyn Fs) -> Result<PathBuf> {
        let bytes = self.load_package(package, fs).await?;
        self.unpack_bytes_to_staging(bytes, &package.display().to_string(), fs)
            .await
    }

    async fn unpack_bytes_to_staging(
        &self,
        bytes: Vec<u8>,
        label: &str,
        fs: &dyn Fs,
    ) -> Result<PathBuf> {
        if bytes.len() as u64 > MAX_PACKAGE_BYTES {
            bail!(
                "{label} is {} bytes; a package may be at most {MAX_PACKAGE_BYTES}",
                bytes.len()
            );
        }
        let staging = self.root.join(STAGING_DIR).join(format!(
            "{}-{}",
            std::process::id(),
            STAGING_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        if fs.metadata(&staging).await?.is_some() {
            remove_dir_checked(&staging, fs).await?;
        }
        fs.create_dir(&staging).await?;
        if let Err(error) = unpack(bytes, &staging, fs).await {
            remove_dir(&staging, fs).await;
            return Err(error.context(format!("unpacking {label}")));
        }
        Ok(staging)
    }
}

static STAGING_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Whether a zip entry name is a plain relative path: normal components only, no drive, root,
/// `..`, backslash or `:` (an NTFS stream).
/// A registry package against its index entry (integration-5 R7).
fn check_matches_release(info: &PackageInfo, release: &Release) -> Result<()> {
    if info.extension_id.as_ref() != release.id.as_str() || info.version != release.version {
        bail!(
            "the package is {} {}, not the release {} {} the registry names",
            info.extension_id,
            info.version,
            release.id,
            release.version
        );
    }
    let manifest = &info.manifest;
    let mut beyond: Vec<String> = manifest
        .capabilities
        .iter()
        .filter(|capability| !release.capabilities.contains(*capability))
        .cloned()
        .collect();
    beyond.extend(
        manifest
            .network
            .iter()
            .flat_map(|network| network.domains.iter())
            .filter(|domain| !release.network_domains.contains(*domain))
            .map(|domain| format!("network:fetch@{domain}")),
    );
    beyond.extend(
        manifest
            .processes
            .iter()
            .filter(|process| !release.processes.contains(&process.name))
            .map(|process| format!("process:spawn@{}", process.name)),
    );
    if !beyond.is_empty() {
        bail!(
            "the package asks for more than its registry entry declares: {}",
            beyond.join(", ")
        );
    }
    Ok(())
}

pub fn is_plain_entry_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('\\')
        && !name.contains(':')
        && Path::new(name.trim_end_matches('/'))
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        && name
            .trim_end_matches('/')
            .split('/')
            .all(|part| !is_windows_alias(part))
}

/// A name Windows rewrites or reserves (integration-5 R9, SEC T10): a trailing dot or space, or
/// a device name (`CON`, `NUL`, `COM1`, …) with or without an extension.
fn is_windows_alias(part: &str) -> bool {
    if part.ends_with('.') || part.ends_with(' ') {
        return true;
    }
    let stem = part.split('.').next().unwrap_or_default().trim_end();
    let upper = stem.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.len() == 4
            && upper[3..].chars().all(|c| c.is_ascii_digit() && c != '0'))
}

async fn unpack(bytes: Vec<u8>, destination: &Path, fs: &dyn Fs) -> Result<()> {
    let reader = async_zip::base::read::mem::ZipFileReader::new(bytes)
        .await
        .context("not a zip archive")?;
    let entries = reader.file().entries().len();
    if entries > MAX_PACKAGE_ENTRIES {
        bail!("{entries} entries; a package may have at most {MAX_PACKAGE_ENTRIES}");
    }
    let mut unpacked: u64 = 0;
    // Names that differ only in case are the same file on Windows and macOS.
    let mut seen = std::collections::HashSet::new();
    for index in 0..entries {
        let entry = &reader.file().entries()[index];
        let name = entry
            .filename()
            .as_str()
            .context("an entry name is not UTF-8")?
            .to_string();
        if !is_plain_entry_name(&name) {
            bail!("entry {name:?} is not a plain relative path");
        }
        if !seen.insert(name.trim_end_matches('/').to_lowercase()) {
            bail!("entry {name:?} appears twice (names that differ only in case count as one)");
        }
        let path = destination.join(name.trim_end_matches('/'));
        if entry
            .dir()
            .with_context(|| format!("reading entry {name:?}"))?
        {
            fs.create_dir(&path).await?;
            continue;
        }
        let remaining = MAX_UNPACKED_BYTES - unpacked;
        if entry.uncompressed_size() > remaining {
            bail!("unpacks to more than {MAX_UNPACKED_BYTES} bytes");
        }
        let mut data = Vec::new();
        reader
            .reader_without_entry(index)
            .await
            .with_context(|| format!("reading entry {name:?}"))?
            // One byte past the limit tells a lying size header from an exact fit.
            .take(remaining + 1)
            .read_to_end(&mut data)
            .await
            .with_context(|| format!("reading entry {name:?}"))?;
        unpacked += data.len() as u64;
        if unpacked > MAX_UNPACKED_BYTES {
            bail!("unpacks to more than {MAX_UNPACKED_BYTES} bytes");
        }
        if let Some(parent) = path.parent() {
            fs.create_dir(parent).await?;
        }
        fs.write(&path, &data)
            .await
            .with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(())
}

/// Reads and checks an unpacked package.
async fn read_package_dir(dir: &Path, fs: &dyn Fs) -> Result<PackageInfo> {
    let Some((extension_id, manifest)) = UiManifest::read(fs, dir).await? else {
        bail!("its extension.toml has no [fyx_ui] table");
    };
    let text = fs.load(&dir.join("extension.toml")).await?;
    let version: VersionToml = toml::from_str(&text).context("reading the version")?;
    let version = Version::parse(&version.version).with_context(|| {
        format!(
            "{extension_id}: version {:?} is not semver",
            version.version
        )
    })?;
    let component = dir.join(&manifest.component);
    let component_size = fs
        .metadata(&component)
        .await?
        .filter(|metadata| !metadata.is_dir)
        .with_context(|| {
            format!(
                "{extension_id}: the component {} is missing",
                manifest.component
            )
        })?
        .len;
    if component_size > MAX_COMPONENT_BYTES {
        bail!(
            "{extension_id}: the component is {component_size} bytes; at most \
             {MAX_COMPONENT_BYTES} are allowed"
        );
    }
    Ok(PackageInfo {
        extension_id,
        version,
        manifest,
        zed_extension: crate::install::has_zed_extension(dir, fs).await,
    })
}

async fn remove_dir_checked(dir: &Path, fs: &dyn Fs) -> Result<()> {
    fs.remove_dir(
        dir,
        RemoveOptions {
            recursive: true,
            ignore_if_not_exists: true,
        },
    )
    .await
    .with_context(|| format!("removing {}", dir.display()))
}

async fn remove_dir(dir: &Path, fs: &dyn Fs) {
    if let Err(error) = remove_dir_checked(dir, fs).await {
        log::warn!("fyx:ui: {error:#}");
    }
}

/// Packages for tests.
#[cfg(test)]
pub(crate) mod test_support {
    use async_zip::{Compression, ZipEntryBuilder, base::write::ZipFileWriter};

    /// A zip of `entries` (stored, not compressed: tests stay fast in debug builds).
    pub(crate) async fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = ZipFileWriter::new(futures::io::Cursor::new(Vec::new()));
        for (name, data) in entries {
            writer
                .write_entry_whole(
                    ZipEntryBuilder::new((*name).to_string().into(), Compression::Stored),
                    data,
                )
                .await
                .expect("zip entry");
        }
        writer.close().await.expect("zip").into_inner()
    }

    pub(crate) fn manifest(id: &str, version: &str, capabilities: &str) -> String {
        format!(
            "id = {id:?}\nname = \"Test\"\nversion = {version:?}\nschema_version = 1\n\n\
             [fyx_ui]\napi_version = \"0.1.0\"\ncomponent = \"fyx_ui.wasm\"\n\
             capabilities = [{capabilities}]\n"
        )
    }

    /// A package of `id` at `version` with `component` as its component.
    pub(crate) async fn package(id: &str, version: &str, component: &[u8]) -> Vec<u8> {
        zip(&[
            (
                "extension.toml",
                manifest(id, version, "\"ui:status-bar\"").as_bytes(),
            ),
            ("fyx_ui.wasm", component),
        ])
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::{test_support::*, *};
    use gpui::TestAppContext;

    struct Setup {
        _root: tempfile::TempDir,
        dir: PathBuf,
        store: InstallStore,
        fs: Arc<dyn Fs>,
    }

    fn setup(cx: &mut TestAppContext) -> Setup {
        cx.executor().allow_parking();
        let root = tempfile::tempdir().expect("temp dir");
        let dir = root.path().to_path_buf();
        Setup {
            store: InstallStore::new(dir.join("data").join("extensions").join("fyx")),
            fs: fs::RealFs::new(None, cx.executor()),
            dir,
            _root: root,
        }
    }

    /// Decision Q1: a package or a dev folder that also holds a Zed extension says so in the
    /// install dialog and on the consent card; one without says nothing.
    #[gpui::test]
    async fn test_a_zed_extension_inside_is_announced(cx: &mut TestAppContext) {
        use crate::install::ZED_EXTENSION_WARNING;
        let setup = setup(cx);
        let fs = setup.fs.as_ref();
        let toml = manifest("both", "1.0.0", "\"ui:panel\"");
        let with = zip(&[
            ("extension.toml", toml.as_bytes()),
            ("fyx_ui.wasm", b"component"),
            ("extension.wasm", b"zed"),
        ])
        .await;
        let without = zip(&[
            ("extension.toml", toml.as_bytes()),
            ("fyx_ui.wasm", b"component"),
        ])
        .await;
        let package = Path::new("both.fyxext");
        let info = setup
            .store
            .inspect_package_bytes(with, "with", fs)
            .await
            .expect("inspects");
        assert!(info.zed_extension);
        assert!(
            crate::store_actions::file_install_detail(&info, package, "")
                .contains(ZED_EXTENSION_WARNING)
        );
        let info = setup
            .store
            .inspect_package_bytes(without, "without", fs)
            .await
            .expect("inspects");
        assert!(!info.zed_extension);
        assert!(
            !crate::store_actions::file_install_detail(&info, package, "")
                .contains(ZED_EXTENSION_WARNING)
        );

        // Any depth or case in a package counts (owner's Q1, `fyx_registry::runs_zed_extension`).
        let nested = zip(&[
            ("extension.toml", toml.as_bytes()),
            ("fyx_ui.wasm", b"component"),
            ("sub/Extension.WASM", b"zed"),
        ])
        .await;
        let info = setup
            .store
            .inspect_package_bytes(nested, "nested", fs)
            .await
            .expect("inspects");
        assert!(info.zed_extension);

        // A dev folder: the install dialog and the consent card of `dev_paths`.
        let folder = setup.dir.join("dev");
        std::fs::create_dir_all(&folder).expect("folder");
        assert!(!crate::install::has_zed_extension(&folder, fs).await);
        // Build output and dependencies are not looked at; any other depth is (integration-6).
        std::fs::create_dir_all(folder.join("target/wasm32-wasip2")).expect("target");
        std::fs::write(folder.join("target/wasm32-wasip2/extension.wasm"), b"zed").expect("built");
        assert!(!crate::install::has_zed_extension(&folder, fs).await);
        std::fs::create_dir_all(folder.join("a/b")).expect("nested");
        std::fs::write(folder.join("a/b/Extension.Wasm"), b"zed").expect("nested zed extension");
        assert!(crate::install::has_zed_extension(&folder, fs).await);
        std::fs::remove_dir_all(folder.join("a")).expect("removed");
        std::fs::write(folder.join("EXTENSION.wasm"), b"zed").expect("zed extension");
        assert!(crate::install::has_zed_extension(&folder, fs).await);
        assert!(
            crate::install::dev_install_detail(&info.manifest, &folder, true)
                .contains(ZED_EXTENSION_WARNING)
        );
        assert!(
            !crate::install::dev_install_detail(&info.manifest, &folder, false)
                .contains(ZED_EXTENSION_WARNING)
        );
        let pending = |zed_extension| crate::registry::PendingConsent {
            extension_id: "both".into(),
            dir: folder.clone(),
            installed_dir: setup.dir.clone(),
            capabilities: vec!["ui:panel".into()],
            is_dev: true,
            new_capabilities: Vec::new(),
            zed_extension,
            record: crate::consent::ConsentRecord {
                capabilities: vec!["ui:panel".into()],
                grant: crate::consent::Grant::DevFolder {
                    path: folder.clone(),
                },
            },
        };
        assert!(crate::registry::consent_card_text(&pending(true)).contains(ZED_EXTENSION_WARNING));
        assert!(
            !crate::registry::consent_card_text(&pending(false)).contains(ZED_EXTENSION_WARNING)
        );
    }

    /// Owner's Q2: a program that can run anything says so wherever it is listed.
    #[test]
    fn test_programs_that_run_any_code_are_named() {
        let items: Vec<String> = [
            "process:args@git status",
            "process:spawn",
            "process:spawn@bash",
            "process:spawn@git",
            "process:spawn@lint (node_modules/.bin/eslint)",
        ]
        .iter()
        .map(|item| item.to_string())
        .collect();
        assert_eq!(crate::install::programs_running_any_code(&items), ["bash"]);
        let lines = crate::install::describe_capability_ids(&items);
        assert!(lines.contains(&format!(
            "Runs bash, which {}",
            ui_ext::process::RUNS_ANY_CODE_PROMPT
        )));
        assert!(lines.contains(&"Runs git".to_string()));
        // Git with an option that sets its configuration can run anything too.
        let configured: Vec<String> = ["process:args@git -c", "process:spawn@git"]
            .iter()
            .map(|item| item.to_string())
            .collect();
        assert_eq!(
            crate::install::programs_running_any_code(&configured),
            ["git"]
        );
    }

    impl Setup {
        fn write_package(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.dir.join(name);
            std::fs::write(&path, bytes).expect("package");
            path
        }

        async fn install(&self, id: &str, version: &str) -> Result<Installed> {
            let bytes = package(id, version, b"component").await;
            let path = self.write_package(&format!("{id}-{version}.fyxext"), &bytes);
            self.store.install_package(&path, self.fs.as_ref()).await
        }

        async fn entry(&self, id: &str) -> Option<ExtensionState> {
            let state = self.store.state(self.fs.as_ref()).await.expect("state");
            state.extensions.get(id).cloned()
        }

        fn versions_on_disk(&self, id: &str) -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(self.store.root().join(id))
                .map(|entries| {
                    entries
                        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default();
            names.sort();
            names
        }

        fn staging_is_empty(&self) -> bool {
            std::fs::read_dir(self.store.root().join(STAGING_DIR))
                .map(|mut entries| entries.next().is_none())
                .unwrap_or(true)
        }
    }

    fn v(version: &str) -> Version {
        Version::parse(version).unwrap()
    }

    /// Plan 1.1: install, update (the replaced version is kept, older ones are deleted with their
    /// precompiled components), roll back and forward, reinstall, uninstall.
    #[gpui::test]
    async fn test_install_update_rollback_uninstall(cx: &mut TestAppContext) {
        let setup = setup(cx);
        let installed = setup.install("demo", "1.0.0").await.expect("installs");
        assert_eq!(installed.previous, None);
        assert_eq!(installed.dir, setup.store.version_dir("demo", &v("1.0.0")));
        assert!(installed.dir.join("extension.toml").is_file());
        assert!(installed.dir.join("fyx_ui.wasm").is_file());
        let entry = setup.entry("demo").await.expect("installed");
        assert_eq!(
            (entry.active, entry.previous, entry.pinned),
            (v("1.0.0"), None, false)
        );
        // Plan 3.5: a file install is recorded as one, with its package's hash.
        let source = &entry.sources["1.0.0"];
        assert_eq!(source.origin, fyx_registry::Origin::LocalFile);
        assert!(source.package_sha256.is_some());

        // A precompiled component of 1.0.0, as a load would write it.
        std::fs::write(
            setup.store.root().join("demo").join("1.0.0-abc.cwasm"),
            b"x",
        )
        .unwrap();
        let installed = setup.install("demo", "2.0.0").await.expect("updates");
        assert_eq!(installed.previous, Some(v("1.0.0")));
        std::fs::write(
            setup.store.root().join("demo").join("2.0.0-abc.cwasm"),
            b"x",
        )
        .unwrap();
        setup.install("demo", "3.0.0").await.expect("updates again");
        assert_eq!(
            setup.versions_on_disk("demo"),
            ["2.0.0", "2.0.0-abc.cwasm", "3.0.0"],
            "1.0.0 and its precompiled component are gone; 2.0.0 is kept for rollback"
        );

        assert_eq!(
            setup
                .store
                .rollback("demo", setup.fs.as_ref())
                .await
                .unwrap(),
            v("2.0.0")
        );
        let entry = setup.entry("demo").await.unwrap();
        assert_eq!(
            (entry.active, entry.previous),
            (v("2.0.0"), Some(v("3.0.0")))
        );
        setup
            .store
            .rollback("demo", setup.fs.as_ref())
            .await
            .unwrap();
        let entry = setup.entry("demo").await.unwrap();
        assert_eq!(
            (entry.active, entry.previous),
            (v("3.0.0"), Some(v("2.0.0")))
        );

        // Reinstalling the active version keeps `previous`.
        setup.install("demo", "3.0.0").await.expect("reinstalls");
        let entry = setup.entry("demo").await.unwrap();
        assert_eq!(
            (entry.active, entry.previous),
            (v("3.0.0"), Some(v("2.0.0")))
        );

        setup
            .store
            .uninstall("demo", setup.fs.as_ref())
            .await
            .unwrap();
        assert_eq!(setup.entry("demo").await, None);
        assert!(!setup.store.root().join("demo").exists());
        assert!(setup.staging_is_empty());
    }

    #[gpui::test]
    async fn test_pinned_version_stays(cx: &mut TestAppContext) {
        let setup = setup(cx);
        setup.install("demo", "1.0.0").await.unwrap();
        setup
            .store
            .set_pinned("demo", true, setup.fs.as_ref())
            .await
            .unwrap();
        let error = setup.install("demo", "2.0.0").await.unwrap_err();
        assert!(
            format!("{error:#}").contains("pinned to 1.0.0"),
            "{error:#}"
        );
        assert_eq!(setup.versions_on_disk("demo"), ["1.0.0"]);
        // The pinned version itself may be reinstalled.
        setup.install("demo", "1.0.0").await.expect("same version");
        setup
            .store
            .set_pinned("demo", false, setup.fs.as_ref())
            .await
            .unwrap();
        setup.install("demo", "2.0.0").await.expect("unpinned");
        assert_eq!(
            setup.entry("demo").await.unwrap().previous,
            Some(v("1.0.0"))
        );
        let error = setup
            .store
            .rollback("nothing", setup.fs.as_ref())
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("not installed"));
    }

    /// integration-5 R9: a rollback clears `disabled` (it was for the other version); uninstall
    /// takes only a valid id; a package with names that differ only in case is refused.
    #[gpui::test]
    async fn test_rollback_uninstall_and_case_duplicates(cx: &mut TestAppContext) {
        let setup = setup(cx);
        setup.install("demo", "1.0.0").await.unwrap();
        setup.install("demo", "2.0.0").await.unwrap();
        setup
            .store
            .set_disabled("demo", Some("blocked: bad".into()), setup.fs.as_ref())
            .await
            .unwrap();
        setup
            .store
            .rollback("demo", setup.fs.as_ref())
            .await
            .unwrap();
        assert_eq!(setup.entry("demo").await.unwrap().disabled, None);

        for bad in ["..", "../outside", "a/b", ""] {
            assert!(
                setup.store.uninstall(bad, setup.fs.as_ref()).await.is_err(),
                "{bad:?}"
            );
        }

        let bytes = zip(&[
            (
                "extension.toml",
                manifest("dup", "1.0.0", "\"ui:status-bar\"").as_bytes(),
            ),
            ("fyx_ui.wasm", b"component"),
            ("assets/Icon.svg", b"<svg/>"),
            ("assets/icon.svg", b"<svg/>"),
        ])
        .await;
        let path = setup.write_package("dup.fyxext", &bytes);
        let error = setup
            .store
            .install_package(&path, setup.fs.as_ref())
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("twice"), "{error:#}");
    }

    #[test]
    fn test_entry_names() {
        for name in ["extension.toml", "assets/icon.svg", "assets/", "a/b/c.txt"] {
            assert!(is_plain_entry_name(name), "{name}");
        }
        for name in [
            "",
            "../evil",
            "a/../../evil",
            "/abs",
            "C:/abs",
            "C:abs",
            r"a\..\..\evil",
            "file.txt:stream",
            "./x/../../y",
            "CON",
            "assets/nul.txt",
            "assets/Com1.svg",
            "lpt9",
            "trailing.",
            "assets/space ",
            "dir./x",
        ] {
            assert!(!is_plain_entry_name(name), "{name}");
        }
        for name in ["console.txt", "assets/com10.svg", "COM0", "nullable"] {
            assert!(is_plain_entry_name(name), "{name}");
        }
    }

    /// Bad packages are refused before anything is installed, and leave no staging folder.
    #[gpui::test]
    async fn test_bad_packages_are_refused(cx: &mut TestAppContext) {
        let setup = setup(cx);
        let good = manifest("demo", "1.0.0", "\"ui:panel\"");
        let big_component = vec![0u8; (MAX_COMPONENT_BYTES + 1) as usize];
        let many: Vec<String> = (0..=MAX_PACKAGE_ENTRIES)
            .map(|index| format!("assets/{index}"))
            .collect();
        let many_entries: Vec<(&str, &[u8])> =
            many.iter().map(|name| (name.as_str(), &b""[..])).collect();
        let bad_id = manifest("x&calc", "1.0.0", "");
        let bad_version = manifest("demo", "one", "");
        let cases: Vec<(&str, Vec<u8>, &str)> = vec![
            (
                "traversal",
                zip(&[
                    ("extension.toml", good.as_bytes()),
                    ("fyx_ui.wasm", b"c"),
                    ("../evil.txt", b"x"),
                ])
                .await,
                "not a plain relative path",
            ),
            (
                "no component",
                zip(&[("extension.toml", good.as_bytes())]).await,
                "missing",
            ),
            (
                "big component",
                zip(&[
                    ("extension.toml", good.as_bytes()),
                    ("fyx_ui.wasm", &big_component),
                ])
                .await,
                "the component is",
            ),
            (
                "bad id",
                zip(&[("extension.toml", bad_id.as_bytes()), ("fyx_ui.wasm", b"c")]).await,
                "id",
            ),
            (
                "bad version",
                zip(&[
                    ("extension.toml", bad_version.as_bytes()),
                    ("fyx_ui.wasm", b"c"),
                ])
                .await,
                "not semver",
            ),
            ("not a zip", b"plain text".to_vec(), "not a zip archive"),
            ("too many entries", zip(&many_entries).await, "entries"),
        ];
        for (name, bytes, expected) in cases {
            let path = setup.write_package(&format!("{name}.fyxext"), &bytes);
            let error = setup
                .store
                .install_package(&path, setup.fs.as_ref())
                .await
                .expect_err(name);
            assert!(format!("{error:#}").contains(expected), "{name}: {error:#}");
            assert!(setup.staging_is_empty(), "{name} left a staging folder");
            assert_eq!(
                setup.entry("demo").await,
                None,
                "{name} installed something"
            );
        }
        assert!(
            !setup
                .dir
                .join("data")
                .join("extensions")
                .join("fyx")
                .join("evil.txt")
                .exists()
        );

        // Over the package size cap: refused before it is read.
        let big = setup.dir.join("big.fyxext");
        std::fs::File::create(&big)
            .unwrap()
            .set_len(MAX_PACKAGE_BYTES + 1)
            .unwrap();
        let error = setup
            .store
            .install_package(&big, setup.fs.as_ref())
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("at most"), "{error:#}");
    }

    #[gpui::test]
    async fn test_inspect_does_not_install(cx: &mut TestAppContext) {
        let setup = setup(cx);
        let bytes = package("demo", "1.2.3", b"c").await;
        let path = setup.write_package("demo.fyxext", &bytes);
        let info = setup
            .store
            .inspect_package(&path, setup.fs.as_ref())
            .await
            .unwrap();
        assert_eq!(
            (info.extension_id.as_ref(), info.version),
            ("demo", v("1.2.3"))
        );
        assert_eq!(info.manifest.capabilities, ["ui:status-bar"]);
        assert_eq!(setup.entry("demo").await, None);
        assert!(setup.staging_is_empty());
    }
}
