//! Consent and grants for UI extensions: the permission broker's store (plan 1.5; it began as
//! the interim guard of integration-4 finding 3).
//!
//! An installed extension loads only with a record for it (see [`ConsentStore::state`]): one
//! written by an install dialog that showed its capabilities (install from file, install dev
//! UI extension), or by one "Enable" for a folder that appeared without a Fyx install action.
//! A dev folder's record names that folder. Beyond that, only T1 capabilities matter: T0 ones
//! (UI surfaces) load silently, so an update that adds only T0 needs nothing, while one that adds a
//! T1 capability is held until the user approves it. Grants can be revoked
//! (`fyx_ui: revoke UI extension permission`); [`ConsentStore::checker`] answers host imports
//! from the current grants, so a revocation applies at once. `FYX_UI_DEV_EXTENSIONS` entries are
//! not checked at load: setting the variable is the opt-in (their T1 calls are still checked).
//!
//! Records live in `<data dir>/fyx-ui-consent.json`.

use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
};

use anyhow::{Context as _, Result};
use fs::Fs;
use gpui::BackgroundExecutor;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use ui_ext_wasm::permissions::{CapabilityCheck, CapabilityDenied, Tier, tier_of};

/// What the user agreed to for one extension.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentRecord {
    /// Sorted and deduplicated.
    pub capabilities: Vec<String>,
    pub grant: Grant,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Grant {
    /// A regular install: this exact component.
    Component { sha256: String },
    /// A dev install: whatever is built in this folder.
    DevFolder { path: PathBuf },
}

impl ConsentRecord {
    /// The record for the extension in `extension_dir` as it is now. The component (up to
    /// 32 MiB) is hashed on `executor`, off the thread that awaits this.
    pub async fn current(
        extension_dir: &Path,
        capabilities: &[String],
        component: &Path,
        is_dev: bool,
        fs: &dyn Fs,
        executor: &BackgroundExecutor,
    ) -> Result<Self> {
        let mut capabilities = capabilities.to_vec();
        capabilities.sort();
        capabilities.dedup();
        let grant = if is_dev {
            Grant::DevFolder {
                path: fs
                    .canonicalize(extension_dir)
                    .await
                    .with_context(|| format!("resolving {}", extension_dir.display()))?,
            }
        } else {
            let bytes = fs
                .load_bytes(component)
                .await
                .with_context(|| format!("reading {}", component.display()))?;
            Grant::Component {
                sha256: executor
                    .spawn(async move { format!("{:x}", Sha256::digest(&bytes)) })
                    .await,
            }
        };
        Ok(Self {
            capabilities,
            grant,
        })
    }

    /// The record for component bytes already read, so what is checked is exactly what loads
    /// (triage-3, SEC T1). Returns the bytes back for the load.
    pub async fn for_bytes(
        extension_dir: &Path,
        capabilities: &[String],
        bytes: Vec<u8>,
        is_dev: bool,
        fs: &dyn Fs,
        executor: &BackgroundExecutor,
    ) -> Result<(Self, Vec<u8>)> {
        let mut capabilities = capabilities.to_vec();
        capabilities.sort();
        capabilities.dedup();
        let (grant, bytes) = if is_dev {
            let path = fs
                .canonicalize(extension_dir)
                .await
                .with_context(|| format!("resolving {}", extension_dir.display()))?;
            (Grant::DevFolder { path }, bytes)
        } else {
            let (sha256, bytes) = executor
                .spawn(async move {
                    let sha256 = format!("{:x}", Sha256::digest(&bytes));
                    (sha256, bytes)
                })
                .await;
            (Grant::Component { sha256 }, bytes)
        };
        Ok((
            Self {
                capabilities,
                grant,
            },
            bytes,
        ))
    }
}

#[derive(Default, Serialize, Deserialize)]
struct ConsentFile {
    extensions: BTreeMap<String, ConsentRecord>,
}

#[derive(Default)]
struct Shared {
    records: BTreeMap<String, ConsentRecord>,
    /// The capabilities each loaded extension declared, by id.
    declared: HashMap<String, Vec<String>>,
}

/// The consent records (the permission broker's grants), kept in memory and, when it has a path,
/// on disk. Shared with [`ConsentStore::checker`], which host imports call from extension threads.
#[derive(Default)]
pub struct ConsentStore {
    path: Option<PathBuf>,
    shared: Arc<Mutex<Shared>>,
    loaded: bool,
}

/// What a record allows, compared with what an extension asks for now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConsentState {
    /// Load it.
    Allowed,
    /// Never enabled (or enabled for another folder): one "Enable" first.
    NotEnabled,
    /// Enabled before, but it now asks for T1 capabilities it was not granted: the update is held
    /// until the user approves them (spec decision 21).
    NeedsApproval { new_capabilities: Vec<String> },
}

impl ConsentStore {
    pub fn at(path: PathBuf) -> Self {
        Self {
            path: Some(path),
            ..Self::default()
        }
    }

    pub fn default_path() -> PathBuf {
        paths::data_dir().join("fyx-ui-consent.json")
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The file still to read, if any: read it with [`read_records`] and pass the result to
    /// [`ConsentStore::set_loaded`].
    pub fn path_to_load(&self) -> Option<PathBuf> {
        if self.loaded {
            return None;
        }
        self.path.clone()
    }

    pub fn set_loaded(&mut self, records: BTreeMap<String, ConsentRecord>) {
        if !self.loaded {
            self.loaded = true;
            let mut shared = self.lock();
            // Grants made before the file was read stay.
            for (id, record) in records {
                shared.records.entry(id).or_insert(record);
            }
        }
    }

    /// Whether an extension may load as it is now (the reconciliation of spec decision 21 and
    /// integration-4 #3): it needs a record (an install dialog, or one "Enable"); a dev folder's
    /// record must name the same folder; every T1 capability it declares must have been granted.
    /// T0 capabilities and the component itself may change freely (updates that add only T0 load
    /// silently).
    pub fn state(&self, extension_id: &str, current: &ConsentRecord) -> ConsentState {
        let shared = self.lock();
        let Some(record) = shared.records.get(extension_id) else {
            return ConsentState::NotEnabled;
        };
        if let Grant::DevFolder { path } = &current.grant
            && record.grant != (Grant::DevFolder { path: path.clone() })
        {
            return ConsentState::NotEnabled;
        }
        let new_capabilities: Vec<String> = current
            .capabilities
            .iter()
            .filter(|capability| tier_of(capability) != Some(Tier::T0))
            .filter(|capability| !record.capabilities.contains(capability))
            .cloned()
            .collect();
        if new_capabilities.is_empty() {
            ConsentState::Allowed
        } else {
            ConsentState::NeedsApproval { new_capabilities }
        }
    }

    /// Records consent and returns the file contents to write, if the store has a file.
    pub fn grant(
        &mut self,
        extension_id: &str,
        record: ConsentRecord,
    ) -> Option<(PathBuf, String)> {
        self.lock().records.insert(extension_id.to_string(), record);
        self.file_contents()
    }

    /// Takes a granted capability back. The checker denies it from the next call on; the next
    /// load asks again. Returns the file contents to write, if the store has a file.
    pub fn revoke(
        &mut self,
        extension_id: &str,
        capability: &str,
    ) -> Result<Option<(PathBuf, String)>> {
        {
            let mut shared = self.lock();
            let Some(record) = shared.records.get_mut(extension_id) else {
                anyhow::bail!("{extension_id} holds no permissions");
            };
            let before = record.capabilities.len();
            record.capabilities.retain(|granted| granted != capability);
            if record.capabilities.len() == before {
                anyhow::bail!("{extension_id} does not hold {capability}");
            }
        }
        Ok(self.file_contents())
    }

    /// Granted capabilities that can be revoked (T1 and up), as (extension id, capability).
    pub fn revocable(&self) -> Vec<(String, String)> {
        self.lock()
            .records
            .iter()
            .flat_map(|(id, record)| {
                record
                    .capabilities
                    .iter()
                    .filter(|capability| tier_of(capability) != Some(Tier::T0))
                    .map(move |capability| (id.clone(), capability.clone()))
            })
            .collect()
    }

    /// Whether `capability` is granted to `extension_id` (T0 needs no grant).
    pub fn is_granted(&self, extension_id: &str, capability: &str) -> bool {
        tier_of(capability) == Some(Tier::T0)
            || self
                .lock()
                .records
                .get(extension_id)
                .is_some_and(|record| record.capabilities.iter().any(|c| c == capability))
    }

    /// What a loaded extension declared, for the checker and the chips.
    pub fn declare(&mut self, extension_id: &str, capabilities: Vec<String>) {
        self.lock()
            .declared
            .insert(extension_id.to_string(), capabilities);
    }

    pub fn declared(&self, extension_id: &str) -> Vec<String> {
        self.lock()
            .declared
            .get(extension_id)
            .cloned()
            .unwrap_or_default()
    }

    /// The broker's check, for `ui_ext_wasm::LoadOptions::capability_check`.
    pub fn checker(&self) -> Arc<dyn CapabilityCheck> {
        Arc::new(Checker(self.shared.clone()))
    }

    fn file_contents(&self) -> Option<(PathBuf, String)> {
        let path = self.path.clone()?;
        let file = ConsentFile {
            extensions: self.lock().records.clone(),
        };
        match serde_json::to_string_pretty(&file) {
            Ok(text) => Some((path, text)),
            Err(error) => {
                log::error!("fyx:ui: cannot save consent: {error}");
                None
            }
        }
    }
}

/// Answers host imports from the store's current grants.
struct Checker(Arc<Mutex<Shared>>);

impl CapabilityCheck for Checker {
    fn check(&self, extension_id: &str, capability: &str) -> Result<(), CapabilityDenied> {
        let shared = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let deny = |reason: &str| {
            Err(CapabilityDenied {
                extension_id: extension_id.to_string(),
                capability: capability.to_string(),
                reason: reason.to_string(),
            })
        };
        let declared = shared
            .declared
            .get(extension_id)
            .is_some_and(|declared| declared.iter().any(|c| c == capability));
        if !declared {
            return deny("not declared in its extension.toml");
        }
        match tier_of(capability) {
            Some(Tier::T0) => Ok(()),
            Some(Tier::T1) => {
                let granted = shared
                    .records
                    .get(extension_id)
                    .is_some_and(|record| record.capabilities.iter().any(|c| c == capability));
                if granted { Ok(()) } else { deny("not granted") }
            }
            Some(Tier::T2) => deny("T2 capabilities are not available yet"),
            None => deny("unknown capability"),
        }
    }
}

/// The records in the consent file. A missing file means no consent yet; a broken one is logged
/// and treated the same, so the user is asked again rather than an extension let through.
pub async fn read_records(path: &Path, fs: &dyn Fs) -> BTreeMap<String, ConsentRecord> {
    let Ok(text) = fs.load(path).await else {
        return BTreeMap::new();
    };
    match serde_json::from_str::<ConsentFile>(&text) {
        Ok(file) => file.extensions,
        Err(error) => {
            log::error!("fyx:ui: ignoring {}: {error}", path.display());
            BTreeMap::new()
        }
    }
}

/// Writes what [`ConsentStore::grant`] returned.
pub async fn save(path: PathBuf, text: String, fs: &dyn Fs) {
    if let Some(dir) = path.parent()
        && let Err(error) = fs.create_dir(dir).await
    {
        log::error!(
            "fyx:ui: cannot save consent to {}: {error:#}",
            path.display()
        );
        return;
    }
    if let Err(error) = fs.atomic_write(path.clone(), text).await {
        log::error!(
            "fyx:ui: cannot save consent to {}: {error:#}",
            path.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(capabilities: &[&str], grant: Grant) -> ConsentRecord {
        ConsentRecord {
            capabilities: capabilities.iter().map(|c| c.to_string()).collect(),
            grant,
        }
    }

    fn component() -> Grant {
        Grant::Component {
            sha256: "abc".into(),
        }
    }

    /// Plan 4.5: the manifest's network domains and programs are consent items, so an update
    /// that adds one, or adds a T2 capability, is held like one that adds T1; the rules for them
    /// are checked when the manifest is read.
    #[test]
    fn test_domains_programs_and_t2_hold_updates() {
        let manifest = |extra: &str| -> ui_ext_wasm::UiManifest {
            toml::from_str(&format!(
                "api_version = \"0.3.0\"
component = \"x.wasm\"
{extra}"
            ))
            .expect("manifest")
        };
        let v1 = manifest(
            "capabilities = [\"ui:panel\", \"network:fetch\"]
network_domains = [\"api.example.com\"]",
        );
        v1.check_network_and_processes().expect("valid");
        let mut store = ConsentStore::default();
        let items = |m: &ui_ext_wasm::UiManifest| {
            let mut items = m.consent_items();
            items.sort();
            items
        };
        let granted = ConsentRecord {
            capabilities: items(&v1),
            grant: component(),
        };
        store.grant("ext", granted.clone());
        assert_eq!(store.state("ext", &granted), ConsentState::Allowed);

        // A new domain: held.
        let v2 = manifest(
            "capabilities = [\"ui:panel\", \"network:fetch\"]
network_domains = [\"api.example.com\", \"*.cdn.example.org\"]",
        );
        v2.check_network_and_processes().expect("valid");
        let update = ConsentRecord {
            capabilities: items(&v2),
            grant: component(),
        };
        assert_eq!(
            store.state("ext", &update),
            ConsentState::NeedsApproval {
                new_capabilities: vec!["network:fetch@*.cdn.example.org".into()]
            }
        );
        // A new T2 capability with its program: held, both listed.
        let v3 = manifest(
            "capabilities = [\"ui:panel\", \"network:fetch\", \"process:spawn\"]
network_domains = [\"api.example.com\"]
[[processes]]
name = \"git\"",
        );
        v3.check_network_and_processes().expect("valid");
        let ConsentState::NeedsApproval { new_capabilities } = store.state(
            "ext",
            &ConsentRecord {
                capabilities: items(&v3),
                grant: component(),
            },
        ) else {
            panic!("held")
        };
        assert_eq!(new_capabilities, ["process:spawn", "process:spawn@git"]);
        assert_eq!(
            crate::install::describe_capability_ids(&new_capabilities)[1],
            "Runs git"
        );

        // The rules.
        for bad in [
            "capabilities = [\"network:fetch\"]
network_domains = [\"Example.com\"]",
            "capabilities = [\"network:fetch\"]
network_domains = [\"example\"]",
            "capabilities = [\"network:fetch\"]
network_domains = [\"example.com:443\"]",
            "capabilities = [\"network:fetch\"]
network_domains = [\"*.*.example.com\"]",
            "network_domains = [\"example.com\"]",
            "capabilities = [\"process:spawn\"]
[[processes]]
name = \"bin/git\"",
            "capabilities = [\"process:spawn\"]
[[processes]]
name = \"..\"",
            "[[processes]]
name = \"git\"",
            "capabilities = [\"process:spawn\"]
[[processes]]
name = \"git\"
env = [\"A=B\"]",
        ] {
            assert!(
                manifest(bad).check_network_and_processes().is_err(),
                "{bad}"
            );
        }
        for good in [
            "capabilities = [\"network:fetch\"]
network_domains = [\"localhost:8080\", \"127.0.0.1\", \"*.example.com\"]",
            "capabilities = [\"process:spawn\"]
[[processes]]
name = \"git\"
env = [\"GIT_DIR\"]",
        ] {
            manifest(good)
                .check_network_and_processes()
                .unwrap_or_else(|error| panic!("{good}: {error:#}"));
        }
    }

    /// The broker's check: T0 needs only the declaration, T1 a grant, and a revocation applies
    /// to the very next check of the same checker.
    #[test]
    fn test_check_and_revoke() {
        let mut store = ConsentStore::default();
        let check = store.checker();
        store.declare(
            "ext",
            vec!["ui:panel".into(), "workspace:read-buffers".into()],
        );
        assert!(check.check("ext", "ui:panel").is_ok());
        assert_eq!(
            check
                .check("ext", "workspace:read-buffers")
                .unwrap_err()
                .reason,
            "not granted"
        );
        assert_eq!(
            check
                .check("ext", "workspace:read-files")
                .unwrap_err()
                .reason,
            "not declared in its extension.toml"
        );
        assert!(check.check("other", "ui:panel").is_err());

        store.grant(
            "ext",
            record(&["ui:panel", "workspace:read-buffers"], component()),
        );
        assert!(check.check("ext", "workspace:read-buffers").is_ok());
        assert_eq!(
            store.revocable(),
            [("ext".to_string(), "workspace:read-buffers".to_string())]
        );
        store.revoke("ext", "workspace:read-buffers").unwrap();
        assert!(check.check("ext", "workspace:read-buffers").is_err());
        assert!(store.revoke("ext", "workspace:read-buffers").is_err());
        assert!(store.revocable().is_empty());
    }

    /// Spec decision 21 with integration-4 #3: a record is needed; then only new T1 capabilities
    /// hold an update; a dev record is for its folder.
    #[test]
    fn test_consent_states() {
        let mut store = ConsentStore::default();
        let now = record(&["ui:panel"], component());
        assert_eq!(store.state("ext", &now), ConsentState::NotEnabled);
        store.grant("ext", now.clone());
        assert_eq!(store.state("ext", &now), ConsentState::Allowed);
        // Another component and more T0: allowed.
        let update = record(
            &["ui:panel", "ui:status-bar"],
            Grant::Component {
                sha256: "def".into(),
            },
        );
        assert_eq!(store.state("ext", &update), ConsentState::Allowed);
        // A new T1: held.
        let update = record(&["ui:panel", "workspace:diagnostics"], component());
        assert_eq!(
            store.state("ext", &update),
            ConsentState::NeedsApproval {
                new_capabilities: vec!["workspace:diagnostics".into()]
            }
        );
        // A dev folder: only the folder it was enabled for.
        let folder = |path: &str| Grant::DevFolder { path: path.into() };
        store.grant("dev", record(&["ui:panel"], folder("C:/a")));
        assert_eq!(
            store.state("dev", &record(&["ui:panel"], folder("C:/a"))),
            ConsentState::Allowed
        );
        assert_eq!(
            store.state("dev", &record(&["ui:panel"], folder("C:/b"))),
            ConsentState::NotEnabled
        );
    }
}
