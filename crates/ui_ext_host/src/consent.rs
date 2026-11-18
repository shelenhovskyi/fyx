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
//! T2 capabilities (plan 4.5, spec decision 21) are asked at first use: the record holds the
//! approved declaration (an update adding one is held like T1), and the user's answer to the
//! first-use prompt is kept beside it ([`ConsentStore::set_t2`]). The checker serves
//! `CapabilityCheck::{state, request, changes}`: `request` joins an open prompt or asks the host
//! to show one ([`ConsentStore::set_prompter`]); a revocation sets `Denied`, and only the
//! extensions page allows it again.
//!
//! Records live in `<data dir>/fyx-ui-consent.json`.

use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
};

use anyhow::{Context as _, Result};
use fs::Fs;
use futures::{
    FutureExt as _,
    channel::{mpsc, oneshot},
    future::BoxFuture,
};
use gpui::BackgroundExecutor;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use ui_ext_wasm::permissions::{CapabilityCheck, CapabilityDenied, PermissionState, Tier, tier_of};

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
    /// The answers to first-use prompts: per extension, T2 capability → allowed.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    t2: BTreeMap<String, BTreeMap<String, bool>>,
}

/// A first-use prompt the host should show (`CapabilityCheck::request` in state `Ask`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptRequest {
    pub extension_id: String,
    pub capability: String,
}

#[derive(Default)]
struct Shared {
    records: BTreeMap<String, ConsentRecord>,
    /// The capabilities each loaded extension declared, by id.
    declared: HashMap<String, Vec<String>>,
    t2: BTreeMap<String, BTreeMap<String, bool>>,
    /// Requests waiting for an open prompt's answer, by (extension, capability).
    waiting: HashMap<(String, String), Vec<oneshot::Sender<PermissionState>>>,
    prompter: Option<mpsc::UnboundedSender<PromptRequest>>,
    subscribers: HashMap<String, Vec<mpsc::UnboundedSender<(String, PermissionState)>>>,
}

impl Shared {
    fn t2_state(&self, extension_id: &str, capability: &str) -> PermissionState {
        let declared = self
            .declared
            .get(extension_id)
            .is_some_and(|declared| declared.iter().any(|c| c == capability));
        if !declared {
            return PermissionState::Unavailable;
        }
        match self
            .t2
            .get(extension_id)
            .and_then(|answers| answers.get(capability))
        {
            Some(true) => PermissionState::Granted,
            Some(false) => PermissionState::Denied,
            None => PermissionState::Ask,
        }
    }

    fn notify(&mut self, extension_id: &str, capability: &str) {
        let state = self.t2_state(extension_id, capability);
        if let Some(subscribers) = self.subscribers.get_mut(extension_id) {
            subscribers.retain(|subscriber| {
                subscriber
                    .unbounded_send((capability.to_string(), state))
                    .is_ok()
            });
        }
    }
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

    pub fn set_loaded(
        &mut self,
        records: BTreeMap<String, ConsentRecord>,
        t2: BTreeMap<String, BTreeMap<String, bool>>,
    ) {
        if !self.loaded {
            self.loaded = true;
            let mut shared = self.lock();
            // Grants made before the file was read stay.
            for (id, record) in records {
                shared.records.entry(id).or_insert(record);
            }
            for (id, answers) in t2 {
                let entry = shared.t2.entry(id).or_default();
                for (capability, allowed) in answers {
                    entry.entry(capability).or_insert(allowed);
                }
            }
        }
    }

    /// Where first-use prompts go. Without one, a request in state `Ask` resolves `Ask`.
    pub fn set_prompter(&mut self, prompter: mpsc::UnboundedSender<PromptRequest>) {
        self.lock().prompter = Some(prompter);
    }

    /// Records the answer to a first-use prompt (`Some`), or resolves the waiting requests
    /// without recording one (`None`: the prompt was closed, or there was no window to show it
    /// in; they get `Denied` and `Ask`). Returns the file contents to write when it recorded.
    pub fn answer_prompt(
        &mut self,
        extension_id: &str,
        capability: &str,
        answer: Option<bool>,
        no_window: bool,
    ) -> Option<(PathBuf, String)> {
        let waiting = {
            let mut shared = self.lock();
            if let Some(allowed) = answer {
                shared
                    .t2
                    .entry(extension_id.to_string())
                    .or_default()
                    .insert(capability.to_string(), allowed);
                shared.notify(extension_id, capability);
            }
            shared
                .waiting
                .remove(&(extension_id.to_string(), capability.to_string()))
                .unwrap_or_default()
        };
        let resolved = match answer {
            Some(true) => PermissionState::Granted,
            Some(false) => PermissionState::Denied,
            None if no_window => PermissionState::Ask,
            None => PermissionState::Denied,
        };
        for waiter in waiting {
            waiter.send(resolved).ok();
        }
        answer.and_then(|_| self.file_contents())
    }

    /// Sets a T2 answer from the extensions page (allow again, or revoke).
    pub fn set_t2(
        &mut self,
        extension_id: &str,
        capability: &str,
        allowed: bool,
    ) -> Option<(PathBuf, String)> {
        self.answer_prompt(extension_id, capability, Some(allowed), false)
    }

    /// The first-use state of a T2 capability for an extension.
    pub fn t2_state(&self, extension_id: &str, capability: &str) -> PermissionState {
        self.lock().t2_state(extension_id, capability)
    }

    /// Whether an extension may load as it is now (the reconciliation of spec decision 21 and
    /// integration-4 #3): it needs a record (an install dialog, or one "Enable"); a dev folder's
    /// record must name the same folder, and a dev folder's record does not count for a regular
    /// install of the same id (integration-5 S5); every T1 capability it declares must have been
    /// granted. T0 capabilities and the component itself may change freely within one install
    /// (updates that add only T0 load silently); an uninstall forgets the record ([`Self::forget`]).
    pub fn state(&self, extension_id: &str, current: &ConsentRecord) -> ConsentState {
        let shared = self.lock();
        let Some(record) = shared.records.get(extension_id) else {
            return ConsentState::NotEnabled;
        };
        match (&current.grant, &record.grant) {
            (Grant::DevFolder { path }, granted)
                if *granted != (Grant::DevFolder { path: path.clone() }) =>
            {
                return ConsentState::NotEnabled;
            }
            (Grant::Component { .. }, Grant::DevFolder { .. }) => return ConsentState::NotEnabled,
            _ => {}
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

    /// An uninstall: the extension's record and first-use answers go, so a later extension with
    /// the same id starts with nothing (integration-5 S5). Requests waiting on a prompt for it
    /// resolve `Denied`. Returns the file contents to write when anything was forgotten.
    pub fn forget(&mut self, extension_id: &str) -> Option<(PathBuf, String)> {
        let (forgotten, waiting) = {
            let mut shared = self.lock();
            let record = shared.records.remove(extension_id).is_some();
            let answers = shared.t2.remove(extension_id);
            let capabilities: Vec<String> = answers
                .iter()
                .flat_map(|answers| answers.keys().cloned())
                .collect();
            for capability in &capabilities {
                shared.notify(extension_id, capability);
            }
            let keys: Vec<(String, String)> = shared
                .waiting
                .keys()
                .filter(|(id, _)| id == extension_id)
                .cloned()
                .collect();
            let waiting: Vec<_> = keys
                .iter()
                .filter_map(|key| shared.waiting.remove(key))
                .flatten()
                .collect();
            (record || answers.is_some(), waiting)
        };
        for waiter in waiting {
            waiter.send(PermissionState::Denied).ok();
        }
        forgotten.then(|| self.file_contents()).flatten()
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
        if tier_of(capability) == Some(Tier::T2) {
            if self.t2_state(extension_id, capability) != PermissionState::Granted {
                anyhow::bail!("{extension_id} does not hold {capability}");
            }
            return Ok(self.set_t2(extension_id, capability, false));
        }
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

    /// Granted capabilities that can be revoked, as (extension id, capability): T1 grants, and T2
    /// capabilities allowed at first use.
    pub fn revocable(&self) -> Vec<(String, String)> {
        let shared = self.lock();
        let t1 = shared.records.iter().flat_map(|(id, record)| {
            record
                .capabilities
                .iter()
                .filter(|capability| tier_of(capability) == Some(Tier::T1))
                .map(move |capability| (id.clone(), capability.clone()))
        });
        let t2 = shared.t2.iter().flat_map(|(id, answers)| {
            answers
                .iter()
                .filter(|(_, allowed)| **allowed)
                .map(move |(capability, _)| (id.clone(), capability.clone()))
        });
        t1.chain(t2).collect()
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
        let file = {
            let shared = self.lock();
            ConsentFile {
                extensions: shared.records.clone(),
                t2: shared.t2.clone(),
            }
        };
        match serde_json::to_string_pretty(&file) {
            Ok(text) => {
                record_newest(&path, &text);
                Some((path, text))
            }
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
            Some(Tier::T2) => match shared.t2_state(extension_id, capability) {
                PermissionState::Granted => Ok(()),
                PermissionState::Ask => deny("ask"),
                PermissionState::Denied => deny("denied"),
                PermissionState::Unavailable => deny("unavailable"),
            },
            None => deny("unknown capability"),
        }
    }

    fn state(&self, extension_id: &str, capability: &str) -> PermissionState {
        let shared = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        match tier_of(capability) {
            Some(Tier::T2) => shared.t2_state(extension_id, capability),
            _ => {
                drop(shared);
                match self.check(extension_id, capability) {
                    Ok(()) => PermissionState::Granted,
                    Err(_) => PermissionState::Denied,
                }
            }
        }
    }

    fn request(&self, extension_id: &str, capability: &str) -> BoxFuture<'static, PermissionState> {
        let mut shared = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let state = match tier_of(capability) {
            Some(Tier::T2) => shared.t2_state(extension_id, capability),
            _ => {
                drop(shared);
                return futures::future::ready(self.state(extension_id, capability)).boxed();
            }
        };
        if state != PermissionState::Ask {
            return futures::future::ready(state).boxed();
        }
        let Some(prompter) = shared.prompter.clone() else {
            return futures::future::ready(PermissionState::Ask).boxed();
        };
        let (sender, receiver) = oneshot::channel();
        let key = (extension_id.to_string(), capability.to_string());
        let first = !shared.waiting.contains_key(&key);
        shared.waiting.entry(key).or_default().push(sender);
        if first
            && prompter
                .unbounded_send(PromptRequest {
                    extension_id: extension_id.to_string(),
                    capability: capability.to_string(),
                })
                .is_err()
        {
            shared
                .waiting
                .remove(&(extension_id.to_string(), capability.to_string()));
            return futures::future::ready(PermissionState::Ask).boxed();
        }
        // A prompt that never answers (its host gone) resolves `Ask`.
        receiver
            .map(|answer| answer.unwrap_or(PermissionState::Ask))
            .boxed()
    }

    fn changes(&self, extension_id: &str) -> mpsc::UnboundedReceiver<(String, PermissionState)> {
        let (sender, receiver) = mpsc::unbounded();
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .subscribers
            .entry(extension_id.to_string())
            .or_default()
            .push(sender);
        receiver
    }
}

/// The records in the consent file. A missing file means no consent yet; a broken one is logged
/// and treated the same, so the user is asked again rather than an extension let through.
/// The records and the first-use answers.
pub async fn read_file(
    path: &Path,
    fs: &dyn Fs,
) -> (
    BTreeMap<String, ConsentRecord>,
    BTreeMap<String, BTreeMap<String, bool>>,
) {
    let Ok(text) = fs.load(path).await else {
        return Default::default();
    };
    match serde_json::from_str::<ConsentFile>(&text) {
        Ok(file) => (file.extensions, file.t2),
        Err(error) => {
            log::error!("fyx:ui: ignoring {}: {error}", path.display());
            Default::default()
        }
    }
}

/// Writes what [`ConsentStore::grant`] returned.
/// The newest contents of each consent file, recorded in order on the main thread when they are
/// made ([`ConsentStore::file_contents`]): saves run in the background and may run in any order,
/// so each one writes the newest, not its own (integration-5 R9).
static NEWEST: std::sync::LazyLock<std::sync::Mutex<HashMap<PathBuf, String>>> =
    std::sync::LazyLock::new(Default::default);
static WRITING: std::sync::LazyLock<futures::lock::Mutex<()>> =
    std::sync::LazyLock::new(Default::default);

fn record_newest(path: &Path, text: &str) {
    NEWEST
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(path.to_path_buf(), text.to_string());
}

/// Writes the consent file: the newest contents made for `path`, or `text` when none was recorded.
pub async fn save(path: PathBuf, text: String, fs: &dyn Fs) {
    let _writing = WRITING.lock().await;
    let text = NEWEST
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&path)
        .cloned()
        .unwrap_or(text);
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

    /// Plan 4.5: T2 at first use. `Ask` until answered; concurrent requests share one prompt; the
    /// answer is recorded and streamed to `changes`; a closed prompt records nothing and gives
    /// `Denied` to the waiting requests; a revoke is `Denied` until the page allows it again; the
    /// answers survive in the consent file.
    #[test]
    fn test_t2_first_use() {
        use futures::{FutureExt as _, StreamExt as _};
        let mut store = ConsentStore::default();
        let checker = store.checker();
        store.declare(
            "ext",
            vec![
                "ui:panel".into(),
                "network:fetch".into(),
                "network:fetch@api.example.com".into(),
            ],
        );
        assert_eq!(checker.state("ext", "network:fetch"), PermissionState::Ask);
        assert_eq!(
            checker.state("ext", "process:spawn"),
            PermissionState::Unavailable,
            "undeclared"
        );
        assert_eq!(
            checker.state("other", "network:fetch"),
            PermissionState::Unavailable
        );
        assert!(checker.check("ext", "network:fetch").is_err());
        // No prompter (no host UI): it stays Ask.
        assert_eq!(
            checker.request("ext", "network:fetch").now_or_never(),
            Some(PermissionState::Ask)
        );

        let (prompter, mut prompts) = futures::channel::mpsc::unbounded();
        store.set_prompter(prompter);
        let mut changes = checker.changes("ext");
        let first = checker.request("ext", "network:fetch");
        let second = checker.request("ext", "network:fetch");
        assert_eq!(
            prompts.next().now_or_never().flatten(),
            Some(PromptRequest {
                extension_id: "ext".into(),
                capability: "network:fetch".into()
            })
        );
        assert!(
            prompts.next().now_or_never().is_none(),
            "one prompt for both"
        );

        // Closed without an answer: both denied, nothing recorded.
        assert!(
            store
                .answer_prompt("ext", "network:fetch", None, false)
                .is_none()
        );
        assert_eq!(first.now_or_never(), Some(PermissionState::Denied));
        assert_eq!(second.now_or_never(), Some(PermissionState::Denied));
        assert_eq!(checker.state("ext", "network:fetch"), PermissionState::Ask);

        // Allowed.
        let third = checker.request("ext", "network:fetch");
        store.answer_prompt("ext", "network:fetch", Some(true), false);
        assert_eq!(third.now_or_never(), Some(PermissionState::Granted));
        assert_eq!(
            checker.state("ext", "network:fetch"),
            PermissionState::Granted
        );
        assert!(checker.check("ext", "network:fetch").is_ok());
        assert_eq!(
            changes.next().now_or_never().flatten(),
            Some(("network:fetch".to_string(), PermissionState::Granted))
        );
        assert_eq!(
            checker.request("ext", "network:fetch").now_or_never(),
            Some(PermissionState::Granted),
            "no prompt once answered"
        );
        assert!(
            store
                .revocable()
                .contains(&("ext".into(), "network:fetch".into()))
        );

        // Revoked: denied at once, and streamed.
        store.revoke("ext", "network:fetch").unwrap();
        assert_eq!(
            checker.state("ext", "network:fetch"),
            PermissionState::Denied
        );
        assert_eq!(
            changes.next().now_or_never().flatten(),
            Some(("network:fetch".to_string(), PermissionState::Denied))
        );
        assert_eq!(
            checker.request("ext", "network:fetch").now_or_never(),
            Some(PermissionState::Denied),
            "a refusal is not asked again"
        );
        assert!(store.revoke("ext", "network:fetch").is_err());
        // The page allows it again.
        store.set_t2("ext", "network:fetch", true);
        assert_eq!(
            checker.state("ext", "network:fetch"),
            PermissionState::Granted
        );

        // The answers are kept in the consent file.
        let mut stored = ConsentStore::at(std::path::PathBuf::from("consent.json"));
        stored.declare("ext", vec!["network:fetch".into()]);
        let (_, text) = stored
            .set_t2("ext", "network:fetch", false)
            .expect("file contents");
        let file: ConsentFile = serde_json::from_str(&text).unwrap();
        assert_eq!(file.t2["ext"]["network:fetch"], false);
    }

    /// Plan 4.5: the manifest's domains, programs, argument patterns and environment names are
    /// consent items (design.md 0.3 §0.6), so an update that adds one, or a T2 capability, is held
    /// like one that adds T1; their rules are checked when the manifest is read.
    #[test]
    fn test_domains_programs_and_t2_hold_updates() {
        let manifest = |extra: &str| -> ui_ext_wasm::UiManifest {
            toml::from_str(&format!(
                "api_version = \"0.3.0\"\ncomponent = \"x.wasm\"\n{extra}"
            ))
            .expect("manifest")
        };
        let check = |manifest: &ui_ext_wasm::UiManifest| {
            manifest
                .check_network()
                .and_then(|()| manifest.check_processes())
        };
        let items = |m: &ui_ext_wasm::UiManifest| {
            let mut items = m.consent_items();
            items.sort();
            items
        };
        let record = |m: &ui_ext_wasm::UiManifest| ConsentRecord {
            capabilities: items(m),
            grant: component(),
        };
        let v1 = manifest(
            "capabilities = [\"ui:panel\", \"network:fetch\"]\n[network]\ndomains = [\"api.example.com\"]",
        );
        check(&v1).expect("valid");
        let mut store = ConsentStore::default();
        store.grant("ext", record(&v1));
        assert_eq!(store.state("ext", &record(&v1)), ConsentState::Allowed);

        // A new domain: held.
        let v2 = manifest(
            "capabilities = [\"ui:panel\", \"network:fetch\"]\n[network]\ndomains = [\"api.example.com\", \"*.cdn.example.org\"]",
        );
        check(&v2).expect("valid");
        assert_eq!(
            store.state("ext", &record(&v2)),
            ConsentState::NeedsApproval {
                new_capabilities: vec!["network:fetch@*.cdn.example.org".into()]
            }
        );
        // A new T2 capability with a program, an argument pattern and an environment name.
        let v3 = manifest(
            "capabilities = [\"ui:panel\", \"network:fetch\", \"process:spawn\"]\n[network]\ndomains = [\"api.example.com\"]\n[[processes]]\nname = \"git\"\nargs = [\"status\"]\n[process]\nenv = [\"NO_COLOR\"]",
        );
        check(&v3).expect("valid");
        let ConsentState::NeedsApproval { new_capabilities } = store.state("ext", &record(&v3))
        else {
            panic!("held")
        };
        assert_eq!(
            new_capabilities,
            [
                "process:args@git status",
                "process:env@NO_COLOR",
                "process:spawn",
                "process:spawn@git"
            ]
        );
        let described = crate::install::describe_capability_ids(&new_capabilities);
        assert!(described.contains(&"Runs git".to_string()));
        // An argument pattern is not one more program.
        assert!(described.contains(&"Runs git with arguments matching status".to_string()));
        assert!(!described.contains(&"Runs git status".to_string()));

        // The rules.
        for bad in [
            "capabilities = [\"network:fetch\"]",
            "[network]\ndomains = [\"example.com\"]",
            "capabilities = [\"network:fetch\"]\n[network]\ndomains = [\"*.com\"]",
            "capabilities = [\"process:spawn\"]\n[[processes]]\nname = \"bin/git\"",
            "capabilities = [\"process:spawn\"]\n[[processes]]\nname = \"..\"",
            "capabilities = [\"process:spawn\"]\n[[processes]]\nname = \"lint\"\npath = \"../x\"",
            "capabilities = [\"process:spawn\"]",
            "[[processes]]\nname = \"git\"",
            "capabilities = [\"process:spawn\"]\n[[processes]]\nname = \"git\"\n[process]\nenv = [\"A=B\"]",
        ] {
            assert!(check(&manifest(bad)).is_err(), "{bad}");
        }
        for good in [
            "capabilities = [\"network:fetch\"]\n[network]\ndomains = [\"localhost:8080\", \"*.example.com\"]",
            "capabilities = [\"process:spawn\"]\n[[processes]]\nname = \"eslint\"\npath = \"node_modules/.bin/eslint\"\n[process]\nenv = [\"NO_COLOR\"]",
        ] {
            check(&manifest(good)).unwrap_or_else(|error| panic!("{good}: {error:#}"));
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

    /// integration-5 S5: an uninstall forgets the record and the first-use answers, so a new
    /// extension with the same id inherits nothing; a dev folder's record does not count for a
    /// regular install of that id.
    #[test]
    fn test_uninstall_forgets_and_sources_do_not_mix() {
        let mut store = ConsentStore::default();
        let checker = store.checker();
        store.declare("ext", vec!["ui:panel".into(), "network:fetch".into()]);
        let installed = record(&["ui:panel", "workspace:diagnostics"], component());
        store.grant("ext", installed.clone());
        store.set_t2("ext", "network:fetch", true);
        assert_eq!(store.state("ext", &installed), ConsentState::Allowed);
        assert_eq!(
            checker.state("ext", "network:fetch"),
            PermissionState::Granted
        );
        let mut changes = checker.changes("ext");

        store.forget("ext");
        assert_eq!(store.state("ext", &installed), ConsentState::NotEnabled);
        assert_eq!(checker.state("ext", "network:fetch"), PermissionState::Ask);
        assert_eq!(
            futures::FutureExt::now_or_never(futures::StreamExt::next(&mut changes)).flatten(),
            Some(("network:fetch".to_string(), PermissionState::Ask)),
            "subscribers hear that it is no longer granted"
        );
        assert!(store.revocable().is_empty());

        store.grant(
            "foo",
            record(
                &["ui:panel"],
                Grant::DevFolder {
                    path: "C:/dev/foo".into(),
                },
            ),
        );
        assert_eq!(
            store.state("foo", &record(&["ui:panel"], component())),
            ConsentState::NotEnabled,
            "a dev folder's consent is not a regular install's"
        );
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
