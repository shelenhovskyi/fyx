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
//! 0.4 (design wit-0.4 §C.2–§C.5) consents per item: the install dialog's checkboxes
//! ([`ConsentStore::install`]) set one program, listener, discovery file or env line at a time, an
//! unticked T2 item is `Denied`, and the extensions page turns items on and off
//! ([`ConsentStore::set_item`]). An item without an answer of its own (a 0.3 install, or the dev
//! path) has its capability's 0.3 state. Updates are compared with the lines the user consented
//! to ([`ConsentStore::diff`]). T3 is asked at the moment of action through
//! [`ConsentStore::set_t3_prompter`], or held by "Always allow" for one component
//! ([`ConsentStore::set_always_allow`]). Changes go to the usage journal
//! ([`crate::journal::JournalStore`]).
//!
//! Records live in `<data dir>/fyx-ui-consent.json`.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
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
use ui_ext_wasm::{
    journal::{Journal as _, JournalKind},
    permissions::{
        CapabilityCheck, CapabilityDenied, ConsentDiff, ConsentItem, HostTerminalId, ItemLine,
        MAX_T3_PROMPTS_PER_EXTENSION, MAX_T3_PROMPTS_PER_TERMINAL, PermissionState, T3Answer,
        T3Capability, T3Prompt, Tier, consent_diff_lines, tier_of,
    },
};

use crate::journal::JournalStore;

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
    /// 0.4: the answers per item, and the lines they answered.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    items: BTreeMap<String, ItemGrants>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    always_allow: BTreeMap<String, AlwaysAllow>,
}

/// One extension's answers per item (0.4, design §C.2).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemGrants {
    /// By [`ConsentItem::key`]: allowed. T1 items are `true` until revoked on the page.
    #[serde(default)]
    pub items: BTreeMap<String, bool>,
    /// The lines the user consented to, which an update is compared with (§C.5).
    #[serde(default)]
    pub granted: Vec<ItemLine>,
}

/// T3 "Always allow" (§C.3), bound to the component it was given for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlwaysAllow {
    pub sha256: String,
    pub capabilities: BTreeSet<T3Capability>,
}

/// The consent file beyond the records, as [`read_file`] returns it for
/// [`ConsentStore::set_loaded`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Answers {
    /// First-use answers: per extension, T2 capability → allowed.
    pub t2: BTreeMap<String, BTreeMap<String, bool>>,
    pub items: BTreeMap<String, ItemGrants>,
    pub always_allow: BTreeMap<String, AlwaysAllow>,
}

impl From<BTreeMap<String, BTreeMap<String, bool>>> for Answers {
    fn from(t2: BTreeMap<String, BTreeMap<String, bool>>) -> Self {
        Self {
            t2,
            ..Self::default()
        }
    }
}

/// A T3 card for the host to show (§C.3): the extension, the card, and where the answer goes.
/// A dropped sender is `Deny`; the runtime drops the receiver when the terminal closes or the
/// extension unloads, which [`oneshot::Sender::cancellation`] reports.
pub type T3PromptRequest = (String, T3Prompt, oneshot::Sender<T3Answer>);

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
    items: BTreeMap<String, ItemGrants>,
    always_allow: BTreeMap<String, AlwaysAllow>,
    t3_prompter: Option<mpsc::UnboundedSender<T3PromptRequest>>,
    /// The terminals of the T3 cards waiting for an answer, per extension.
    t3_pending: HashMap<String, Vec<HostTerminalId>>,
    /// What the user shared from a terminal's menu, or "until it closes" on a card (§C.3).
    shares: HashMap<(String, HostTerminalId), BTreeMap<T3Capability, T3Answer>>,
    /// Dropped when the terminal closes: its open cards end with `Deny`.
    t3_cancels: HashMap<HostTerminalId, Vec<oneshot::Sender<()>>>,
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
        let state = self.key_state(extension_id, capability);
        self.send(extension_id, capability, state);
    }

    fn send(&mut self, extension_id: &str, key: &str, state: PermissionState) {
        if let Some(subscribers) = self.subscribers.get_mut(extension_id) {
            subscribers
                .retain(|subscriber| subscriber.unbounded_send((key.to_string(), state)).is_ok());
        }
    }

    fn declares(&self, extension_id: &str, capability: &str) -> bool {
        self.declared
            .get(extension_id)
            .is_some_and(|declared| declared.iter().any(|c| c == capability))
    }

    /// Whether "Always allow" holds for this component.
    fn always_allows(&self, extension_id: &str, capability: T3Capability) -> bool {
        let Some(always) = self.always_allow.get(extension_id) else {
            return false;
        };
        always.capabilities.contains(&capability)
            && matches!(
                self.records.get(extension_id).map(|record| &record.grant),
                Some(Grant::Component { sha256 }) if *sha256 == always.sha256
            )
    }

    /// A capability's state before items: T1 by the record, T2 by the first-use answers.
    fn legacy_state(&self, extension_id: &str, capability: &str) -> PermissionState {
        if !self.declares(extension_id, capability) {
            return PermissionState::Unavailable;
        }
        match tier_of(capability) {
            Some(Tier::T0) => PermissionState::Granted,
            Some(Tier::T1) => {
                let granted = self
                    .records
                    .get(extension_id)
                    .is_some_and(|record| record.capabilities.iter().any(|c| c == capability));
                if granted {
                    PermissionState::Granted
                } else {
                    PermissionState::Denied
                }
            }
            Some(Tier::T2) => self.t2_state(extension_id, capability),
            Some(Tier::T3) => self.t3_state(extension_id, capability),
            None => PermissionState::Unavailable,
        }
    }

    /// T3 is asked each time (`Ask`), unless "Always allow" holds.
    fn t3_state(&self, extension_id: &str, capability: &str) -> PermissionState {
        match T3Capability::from_name(capability) {
            Some(t3) if self.always_allows(extension_id, t3) => PermissionState::Granted,
            Some(_) => PermissionState::Ask,
            None => PermissionState::Unavailable,
        }
    }

    fn explicit(&self, extension_id: &str, key: &str) -> Option<bool> {
        self.items
            .get(extension_id)
            .and_then(|grants| grants.items.get(key))
            .copied()
    }

    /// The state of a capability (`network:fetch`) or of one item (`process:spawn#codex`).
    fn key_state(&self, extension_id: &str, key: &str) -> PermissionState {
        match key.split_once('#') {
            None => self.capability_state(extension_id, key),
            Some((capability, _)) => self.item_key_state(extension_id, key, capability),
        }
    }

    fn item_key_state(&self, extension_id: &str, key: &str, capability: &str) -> PermissionState {
        if !self.declares(extension_id, capability) {
            return PermissionState::Unavailable;
        }
        match tier_of(capability) {
            Some(Tier::T0) => PermissionState::Granted,
            Some(Tier::T3) => self.t3_state(extension_id, capability),
            None => PermissionState::Unavailable,
            Some(Tier::T1 | Tier::T2) => match self.explicit(extension_id, key) {
                Some(true) => PermissionState::Granted,
                Some(false) => PermissionState::Denied,
                None => self.legacy_state(extension_id, capability),
            },
        }
    }

    /// A capability with items: its own line decides when it has one (`network:fetch`,
    /// `terminal:env`); otherwise it is granted when any of its items is (`process:spawn`).
    fn capability_state(&self, extension_id: &str, capability: &str) -> PermissionState {
        if !matches!(tier_of(capability), Some(Tier::T1 | Tier::T2))
            || !self.declares(extension_id, capability)
        {
            return self.legacy_state(extension_id, capability);
        }
        let Some(grants) = self.items.get(extension_id) else {
            return self.legacy_state(extension_id, capability);
        };
        if let Some(allowed) = grants.items.get(capability) {
            return if *allowed {
                PermissionState::Granted
            } else {
                PermissionState::Denied
            };
        }
        let prefix = format!("{capability}#");
        let keys: BTreeSet<String> = grants
            .items
            .keys()
            .filter(|key| key.starts_with(&prefix))
            .cloned()
            .chain(
                grants
                    .granted
                    .iter()
                    .map(|line| line.item.key())
                    .filter(|key| key.starts_with(&prefix)),
            )
            .collect();
        if keys.is_empty() {
            return self.legacy_state(extension_id, capability);
        }
        let states: Vec<PermissionState> = keys
            .iter()
            .map(|key| self.item_key_state(extension_id, key, capability))
            .collect();
        if states.contains(&PermissionState::Granted) {
            PermissionState::Granted
        } else if states.contains(&PermissionState::Ask) {
            PermissionState::Ask
        } else {
            PermissionState::Denied
        }
    }

    /// Every capability and item key whose state can change for an extension.
    fn keys_of(&self, extension_id: &str) -> BTreeSet<String> {
        let mut keys = BTreeSet::new();
        if let Some(declared) = self.declared.get(extension_id) {
            keys.extend(
                declared
                    .iter()
                    .filter(|capability| tier_of(capability).is_some())
                    .cloned(),
            );
        }
        if let Some(grants) = self.items.get(extension_id) {
            keys.extend(grants.items.keys().cloned());
            keys.extend(grants.granted.iter().map(|line| line.item.key()));
        }
        let capabilities: Vec<String> = keys
            .iter()
            .filter_map(|key| {
                key.split_once('#')
                    .map(|(capability, _)| capability.to_string())
            })
            .collect();
        keys.extend(capabilities);
        keys
    }

    /// The lines an update is compared with: those consented to, or for a 0.3 record, the new
    /// lines of the capabilities it holds, unchanged.
    fn granted_lines(&self, extension_id: &str, lines: &[ItemLine]) -> Vec<ItemLine> {
        if let Some(grants) = self.items.get(extension_id)
            && !grants.granted.is_empty()
        {
            return grants.granted.clone();
        }
        let Some(record) = self.records.get(extension_id) else {
            return Vec::new();
        };
        lines
            .iter()
            .filter(|line| {
                line.tier == Tier::T0
                    || record
                        .capabilities
                        .iter()
                        .any(|capability| capability == line.item.capability())
            })
            .cloned()
            .collect()
    }
}

/// A T3 card waiting for its answer; dropping it frees its place under the caps.
struct PendingT3 {
    shared: Arc<Mutex<Shared>>,
    extension_id: String,
    terminal: HostTerminalId,
}

impl Drop for PendingT3 {
    fn drop(&mut self) {
        let mut shared = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(pending) = shared.t3_pending.get_mut(&self.extension_id) {
            if let Some(index) = pending
                .iter()
                .position(|terminal| *terminal == self.terminal)
            {
                pending.swap_remove(index);
            }
            if pending.is_empty() {
                shared.t3_pending.remove(&self.extension_id);
            }
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
    journal: Option<Arc<JournalStore>>,
}

/// What a record allows, compared with what an extension asks for now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConsentState {
    /// Load it.
    Allowed,
    /// Never enabled (or enabled for another folder): one "Enable" first.
    NotEnabled,
    /// Enabled before, but it now asks for T1 capabilities it was not granted: the update is held
    /// until the user approves them (spec decision 21). With the lines of the new manifest
    /// ([`ConsentStore::state_with`]), `diff` holds what it adds or widens (§C.5).
    NeedsApproval {
        new_capabilities: Vec<String>,
        diff: ConsentDiff,
    },
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
        answers: impl Into<Answers>,
    ) {
        if !self.loaded {
            self.loaded = true;
            let answers = answers.into();
            let mut shared = self.lock();
            // Grants made before the file was read stay.
            for (id, record) in records {
                shared.records.entry(id).or_insert(record);
            }
            for (id, answers) in answers.t2 {
                let entry = shared.t2.entry(id).or_default();
                for (capability, allowed) in answers {
                    entry.entry(capability).or_insert(allowed);
                }
            }
            for (id, grants) in answers.items {
                shared.items.entry(id).or_insert(grants);
            }
            for (id, always) in answers.always_allow {
                shared.always_allow.entry(id).or_insert(always);
            }
        }
    }

    /// Where consent changes are journaled, and the journal an uninstall clears.
    pub fn set_journal(&mut self, journal: Arc<JournalStore>) {
        self.journal = Some(journal);
    }

    fn journal(&self, extension_id: &str, kind: JournalKind) {
        if let Some(journal) = &self.journal {
            journal.record(extension_id, kind);
        }
    }

    /// Where T3 cards go (§C.3). Without one, every T3 request is `Deny`.
    pub fn set_t3_prompter(&mut self, prompter: mpsc::UnboundedSender<T3PromptRequest>) {
        self.lock().t3_prompter = Some(prompter);
    }

    /// Runs `change` and streams every capability and item whose state it changed.
    fn change<R>(&self, extension_id: &str, change: impl FnOnce(&mut Shared) -> R) -> R {
        let mut shared = self.lock();
        let before: BTreeMap<String, PermissionState> = shared
            .keys_of(extension_id)
            .into_iter()
            .map(|key| {
                let state = shared.key_state(extension_id, &key);
                (key, state)
            })
            .collect();
        let result = change(&mut shared);
        let mut keys = shared.keys_of(extension_id);
        keys.extend(before.keys().cloned());
        // Items first, so the runtime stops an item's work before it hears of its capability.
        let (items, capabilities): (Vec<String>, Vec<String>) =
            keys.into_iter().partition(|key| key.contains('#'));
        for key in items.into_iter().chain(capabilities) {
            let state = shared.key_state(extension_id, &key);
            if before.get(&key).is_none_or(|before| *before != state) {
                shared.send(extension_id, &key, state);
            }
        }
        result
    }

    /// An install or an approved update from a dialog that showed `lines`
    /// (`ui_ext_wasm::permissions::consent_items` of its manifest): T1 items are granted (a T1
    /// item revoked on the page stays revoked), T2 items are granted when `ticked` and `Denied`
    /// when not; an item the update widens, or one already `Denied`, is `Denied` unless ticked;
    /// an unchanged item keeps its answer. Returns the file contents to write.
    pub fn install(
        &mut self,
        extension_id: &str,
        record: ConsentRecord,
        lines: Vec<ItemLine>,
        ticked: &[ConsentItem],
    ) -> Option<(PathBuf, String)> {
        let widened: BTreeSet<ConsentItem> = {
            let shared = self.lock();
            let granted = shared.granted_lines(extension_id, &lines);
            consent_diff_lines(&granted, &lines)
                .widened
                .into_iter()
                .map(|widened| widened.line.item)
                .collect()
        };
        let changed = self.change(extension_id, |shared| {
            let previous = shared
                .items
                .get(extension_id)
                .map(|grants| grants.items.clone())
                .unwrap_or_default();
            let mut items = BTreeMap::new();
            let mut changed = Vec::new();
            for line in &lines {
                let key = line.item.key();
                let allowed = match line.tier {
                    Tier::T0 | Tier::T3 => continue,
                    Tier::T1 => previous.get(&key).copied().unwrap_or(true),
                    Tier::T2 => {
                        if ticked.contains(&line.item) {
                            true
                        } else if widened.contains(&line.item) {
                            false
                        } else {
                            previous.get(&key).copied().unwrap_or(false)
                        }
                    }
                };
                if line.tier == Tier::T2
                    && previous.get(&key) != Some(&allowed)
                    && (allowed || previous.contains_key(&key))
                {
                    changed.push((key.clone(), allowed));
                }
                items.insert(key, allowed);
            }
            shared.records.insert(extension_id.to_string(), record);
            shared.items.insert(
                extension_id.to_string(),
                ItemGrants {
                    items,
                    granted: lines,
                },
            );
            changed
        });
        for (item_key, allowed) in changed {
            self.journal(
                extension_id,
                JournalKind::ConsentChanged { item_key, allowed },
            );
        }
        self.file_contents()
    }

    /// What an update with these lines adds or widens against what was consented to (§C.5). A
    /// widened item that is `Denied` is not in it: it installs silently and stays `Denied`.
    pub fn diff(&self, extension_id: &str, lines: &[ItemLine]) -> ConsentDiff {
        let shared = self.lock();
        let granted = shared.granted_lines(extension_id, lines);
        let mut diff = consent_diff_lines(&granted, lines);
        diff.widened.retain(|widened| {
            shared.explicit(extension_id, &widened.line.item.key()) != Some(false)
        });
        diff
    }

    /// [`Self::state`] with the lines of the manifest that loads: an update that adds or widens
    /// an item is held too, with its diff.
    pub fn state_with(
        &self,
        extension_id: &str,
        current: &ConsentRecord,
        lines: &[ItemLine],
    ) -> ConsentState {
        let new_capabilities = match self.state(extension_id, current) {
            ConsentState::NotEnabled => return ConsentState::NotEnabled,
            ConsentState::Allowed => Vec::new(),
            ConsentState::NeedsApproval {
                new_capabilities, ..
            } => new_capabilities,
        };
        let diff = self.diff(extension_id, lines);
        if new_capabilities.is_empty() && !diff.needs_approval() {
            ConsentState::Allowed
        } else {
            ConsentState::NeedsApproval {
                new_capabilities,
                diff,
            }
        }
    }

    /// The extensions page's switch for one item, T1 or T2 (T0 and T3 have none). Turning one
    /// off stops its work at once: the runtime hears of it through `changes`. Returns the file
    /// contents to write.
    pub fn set_item(
        &mut self,
        extension_id: &str,
        item: &ConsentItem,
        allowed: bool,
    ) -> Result<Option<(PathBuf, String)>> {
        if !matches!(item.tier(), Some(Tier::T1 | Tier::T2)) {
            anyhow::bail!("{} has no switch", item.key());
        }
        if !self.lock().records.contains_key(extension_id) {
            anyhow::bail!("{extension_id} holds no permissions");
        }
        let key = item.key();
        let changed = self.change(extension_id, |shared| {
            let grants = shared.items.entry(extension_id.to_string()).or_default();
            grants.items.insert(key.clone(), allowed) != Some(allowed)
        });
        if changed {
            self.journal(
                extension_id,
                JournalKind::ConsentChanged {
                    item_key: key,
                    allowed,
                },
            );
        }
        Ok(self.file_contents())
    }

    /// One item's state now.
    pub fn item_state(&self, extension_id: &str, item: &ConsentItem) -> PermissionState {
        let shared = self.lock();
        match item {
            ConsentItem::Capability(capability) => {
                shared.capability_state(extension_id, capability)
            }
            _ => shared.item_key_state(extension_id, &item.key(), item.capability()),
        }
    }

    /// The consented lines' items with their states, for the extensions page. Empty for an
    /// extension installed before 0.4: ask [`Self::item_state`] for its manifest's items.
    pub fn items(&self, extension_id: &str) -> Vec<(ConsentItem, PermissionState)> {
        let lines: Vec<ItemLine> = self
            .lock()
            .items
            .get(extension_id)
            .map(|grants| grants.granted.clone())
            .unwrap_or_default();
        lines
            .into_iter()
            .map(|line| {
                let state = self.item_state(extension_id, &line.item);
                (line.item, state)
            })
            .collect()
    }

    /// Whether "Always allow" can be given: only to a component with a hash, not a dev folder.
    pub fn can_always_allow(&self, extension_id: &str) -> bool {
        matches!(
            self.lock()
                .records
                .get(extension_id)
                .map(|record| &record.grant),
            Some(Grant::Component { .. })
        )
    }

    /// T3 "Always allow" on or off (§C.3), for the component installed now: a new sha256
    /// resets it. The runtime still refuses T3 in an untrusted worktree. Returns the file
    /// contents to write.
    pub fn set_always_allow(
        &mut self,
        extension_id: &str,
        capability: T3Capability,
        on: bool,
    ) -> Result<Option<(PathBuf, String)>> {
        let sha256 = {
            let shared = self.lock();
            let Some(record) = shared.records.get(extension_id) else {
                anyhow::bail!("{extension_id} is not installed");
            };
            if on && !record.capabilities.iter().any(|c| c == capability.name()) {
                anyhow::bail!("{extension_id} does not ask for {}", capability.name());
            }
            match &record.grant {
                Grant::Component { sha256 } => sha256.clone(),
                Grant::DevFolder { .. } => anyhow::bail!(
                    "{extension_id} is a dev folder: \"Always allow\" needs a fixed component"
                ),
            }
        };
        let changed = self.change(extension_id, |shared| {
            let before = shared.always_allows(extension_id, capability);
            let always = shared
                .always_allow
                .entry(extension_id.to_string())
                .or_insert_with(|| AlwaysAllow {
                    sha256: sha256.clone(),
                    capabilities: BTreeSet::new(),
                });
            if always.sha256 != sha256 {
                always.sha256 = sha256;
                always.capabilities.clear();
            }
            if on {
                always.capabilities.insert(capability);
            } else {
                always.capabilities.remove(&capability);
            }
            if always.capabilities.is_empty() {
                shared.always_allow.remove(extension_id);
            }
            before != on
        });
        if changed {
            self.journal(
                extension_id,
                JournalKind::ConsentChanged {
                    item_key: format!("{}#always", capability.name()),
                    allowed: on,
                },
            );
        }
        Ok(self.file_contents())
    }

    /// Every "Always allow" that holds now, for the status bar and the terminal tabs.
    pub fn always_allowed(&self) -> Vec<(String, T3Capability)> {
        let shared = self.lock();
        shared
            .always_allow
            .iter()
            .flat_map(|(id, always)| {
                always
                    .capabilities
                    .iter()
                    .map(move |capability| (id.clone(), *capability))
            })
            .filter(|(id, capability)| shared.always_allows(id, *capability))
            .collect()
    }

    /// The terminal menu's "Share with <extension>…" (§C.3): for reading, the next request
    /// (`Once`) or every request until the terminal closes (`UntilClosed`) needs no card; for
    /// writing, it opens writes, each still confirmed on its card. `Deny` takes it back.
    pub fn share_terminal(
        &mut self,
        extension_id: &str,
        terminal: HostTerminalId,
        capability: T3Capability,
        answer: T3Answer,
    ) {
        {
            let mut shared = self.lock();
            let key = (extension_id.to_string(), terminal);
            let shares = shared.shares.entry(key.clone()).or_default();
            match answer {
                T3Answer::Deny => {
                    shares.remove(&capability);
                }
                T3Answer::Once | T3Answer::UntilClosed => {
                    shares.insert(capability, answer);
                }
            }
            if shares.is_empty() {
                shared.shares.remove(&key);
            }
        }
        self.journal(
            extension_id,
            JournalKind::ConsentChanged {
                item_key: format!("{}#shared", capability.name()),
                allowed: answer != T3Answer::Deny,
            },
        );
    }

    /// Everything shared from one terminal with one extension goes.
    pub fn unshare(&mut self, extension_id: &str, terminal: HostTerminalId) {
        let removed = self
            .lock()
            .shares
            .remove(&(extension_id.to_string(), terminal));
        for capability in removed.into_iter().flat_map(|shares| shares.into_keys()) {
            self.journal(
                extension_id,
                JournalKind::ConsentChanged {
                    item_key: format!("{}#shared", capability.name()),
                    allowed: false,
                },
            );
        }
    }

    /// A terminal closed: its shares go and its open cards end with `Deny`.
    pub fn terminal_closed(&mut self, terminal: HostTerminalId) {
        let mut shared = self.lock();
        shared
            .shares
            .retain(|(_, shared_terminal), _| *shared_terminal != terminal);
        shared.t3_cancels.remove(&terminal);
    }

    /// Who may use a terminal now, for its tab marker: what was shared from it (an "Always
    /// allow" holds for every terminal: [`Self::always_allowed`]).
    pub fn shared(&self, terminal: HostTerminalId) -> Vec<(String, T3Capability)> {
        let shared = self.lock();
        let mut shares: Vec<(String, T3Capability)> = shared
            .shares
            .iter()
            .filter(|((_, shared_terminal), _)| *shared_terminal == terminal)
            .flat_map(|((id, _), shares)| {
                shares
                    .iter()
                    .filter(|(_, answer)| **answer != T3Answer::Deny)
                    .map(|(capability, _)| (id.clone(), *capability))
            })
            .collect();
        shares.sort();
        shares
    }

    /// Every extension's state changes, for the page, the status bar and the tab markers.
    pub fn changes(
        &self,
        extension_id: &str,
    ) -> mpsc::UnboundedReceiver<(String, PermissionState)> {
        self.checker().changes(extension_id)
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
            ConsentState::NeedsApproval {
                new_capabilities,
                diff: ConsentDiff::default(),
            }
        }
    }

    /// An uninstall: the extension's record and first-use answers go, so a later extension with
    /// the same id starts with nothing (integration-5 S5). Requests waiting on a prompt for it
    /// resolve `Denied`. Returns the file contents to write when anything was forgotten.
    pub fn forget(&mut self, extension_id: &str) -> Option<(PathBuf, String)> {
        let (forgotten, waiting) = {
            let mut shared = self.lock();
            let mut keys = shared.keys_of(extension_id);
            let record = shared.records.remove(extension_id).is_some();
            let answers = shared.t2.remove(extension_id);
            let items = shared.items.remove(extension_id).is_some();
            let always = shared.always_allow.remove(extension_id).is_some();
            shared.shares.retain(|(id, _), _| id != extension_id);
            keys.extend(answers.iter().flat_map(|answers| answers.keys().cloned()));
            // Items first, as in `change`.
            let (items_keys, capabilities): (Vec<String>, Vec<String>) =
                keys.into_iter().partition(|key| key.contains('#'));
            for key in items_keys.iter().chain(&capabilities) {
                shared.notify(extension_id, key);
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
            (record || answers.is_some() || items || always, waiting)
        };
        for waiter in waiting {
            waiter.send(PermissionState::Denied).ok();
        }
        // §C.4: the journal is cleared on uninstall.
        if let Some(journal) = &self.journal {
            journal.clear_later(extension_id);
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
        let item_keys: Vec<String> = {
            let shared = self.lock();
            let prefix = format!("{capability}#");
            shared
                .items
                .get(extension_id)
                .map(|grants| {
                    grants
                        .items
                        .iter()
                        .filter(|(key, allowed)| {
                            **allowed && (key.as_str() == capability || key.starts_with(&prefix))
                        })
                        .map(|(key, _)| key.clone())
                        .collect()
                })
                .unwrap_or_default()
        };
        if !item_keys.is_empty() {
            self.change(extension_id, |shared| {
                let grants = shared.items.entry(extension_id.to_string()).or_default();
                for key in &item_keys {
                    grants.items.insert(key.clone(), false);
                }
            });
            for item_key in item_keys {
                self.journal(
                    extension_id,
                    JournalKind::ConsentChanged {
                        item_key,
                        allowed: false,
                    },
                );
            }
            return Ok(self.file_contents());
        }
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
        let items = shared.items.iter().flat_map(|(id, grants)| {
            grants
                .items
                .iter()
                .filter(|(_, allowed)| **allowed)
                .map(move |(key, _)| {
                    let capability = key.split_once('#').map_or(key.as_str(), |(c, _)| c);
                    (id.clone(), capability.to_string())
                })
        });
        let mut revocable: Vec<(String, String)> = t1.chain(t2).chain(items).collect();
        revocable.sort();
        revocable.dedup();
        revocable
    }

    /// Whether `capability` is granted to `extension_id` (T0 needs no grant).
    pub fn is_granted(&self, extension_id: &str, capability: &str) -> bool {
        let shared = self.lock();
        tier_of(capability) == Some(Tier::T0)
            || (shared.explicit(extension_id, capability) != Some(false)
                && shared
                    .records
                    .get(extension_id)
                    .is_some_and(|record| record.capabilities.iter().any(|c| c == capability)))
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
                items: shared.items.clone(),
                always_allow: shared.always_allow.clone(),
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
        if !shared.declares(extension_id, capability) {
            return deny("not declared in its extension.toml");
        }
        match tier_of(capability) {
            Some(Tier::T0) => Ok(()),
            Some(Tier::T1) => match shared.capability_state(extension_id, capability) {
                PermissionState::Granted => Ok(()),
                _ => deny("not granted"),
            },
            Some(Tier::T2) => match shared.capability_state(extension_id, capability) {
                PermissionState::Granted => Ok(()),
                PermissionState::Ask => deny("ask"),
                PermissionState::Denied => deny("denied"),
                PermissionState::Unavailable => deny("unavailable"),
            },
            // Asked at the moment of action (`t3_request`), unless "Always allow" holds.
            Some(Tier::T3) => match shared.t3_state(extension_id, capability) {
                PermissionState::Granted => Ok(()),
                _ => deny("ask"),
            },
            None => deny("unknown capability"),
        }
    }

    fn state(&self, extension_id: &str, capability: &str) -> PermissionState {
        let shared = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        match tier_of(capability) {
            Some(Tier::T2 | Tier::T3) => shared.capability_state(extension_id, capability),
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
            Some(Tier::T2) => shared.capability_state(extension_id, capability),
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

    fn item_state(&self, extension_id: &str, item: &ConsentItem) -> PermissionState {
        let shared = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        match item {
            ConsentItem::Capability(capability) => {
                shared.capability_state(extension_id, capability)
            }
            _ => shared.item_key_state(extension_id, &item.key(), item.capability()),
        }
    }

    /// §C.3: "Always allow" answers `Once` at once; otherwise a card goes to the T3 prompter,
    /// within the caps (one per terminal, four per extension; past them `Deny` with no card).
    fn t3_request(&self, extension_id: &str, prompt: T3Prompt) -> BoxFuture<'static, T3Answer> {
        let mut shared = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let capability = prompt.capability;
        if !shared.declares(extension_id, capability.name()) {
            return futures::future::ready(T3Answer::Deny).boxed();
        }
        if shared.always_allows(extension_id, capability) {
            return futures::future::ready(T3Answer::Once).boxed();
        }
        let share_key = (extension_id.to_string(), prompt.terminal);
        let shared_as = shared
            .shares
            .get(&share_key)
            .and_then(|shares| shares.get(&capability))
            .copied();
        match (capability, shared_as) {
            // Shared for reading: no card, once or until the terminal closes.
            (T3Capability::Read, Some(T3Answer::UntilClosed)) => {
                return futures::future::ready(T3Answer::Once).boxed();
            }
            (T3Capability::Read, Some(T3Answer::Once)) => {
                if let Some(shares) = shared.shares.get_mut(&share_key) {
                    shares.remove(&capability);
                    if shares.is_empty() {
                        shared.shares.remove(&share_key);
                    }
                }
                return futures::future::ready(T3Answer::Once).boxed();
            }
            // Writes only on a terminal shared for writing, each one on its card.
            (T3Capability::Write, None | Some(T3Answer::Deny)) => {
                return futures::future::ready(T3Answer::Deny).boxed();
            }
            _ => {}
        }
        let Some(prompter) = shared.t3_prompter.clone() else {
            return futures::future::ready(T3Answer::Deny).boxed();
        };
        let terminal = prompt.terminal;
        let pending = shared
            .t3_pending
            .entry(extension_id.to_string())
            .or_default();
        if pending.len() >= MAX_T3_PROMPTS_PER_EXTENSION
            || pending.iter().filter(|open| **open == terminal).count()
                >= MAX_T3_PROMPTS_PER_TERMINAL
        {
            return futures::future::ready(T3Answer::Deny).boxed();
        }
        pending.push(terminal);
        let (cancel, cancelled) = oneshot::channel::<()>();
        let cancels = shared.t3_cancels.entry(terminal).or_default();
        cancels.retain(|cancel| !cancel.is_canceled());
        cancels.push(cancel);
        drop(shared);
        let guard = PendingT3 {
            shared: self.0.clone(),
            extension_id: extension_id.to_string(),
            terminal,
        };
        let (sender, receiver) = oneshot::channel();
        if prompter
            .unbounded_send((extension_id.to_string(), prompt, sender))
            .is_err()
        {
            drop(guard);
            return futures::future::ready(T3Answer::Deny).boxed();
        }
        async move {
            let answer = match futures::future::select(receiver, cancelled).await {
                futures::future::Either::Left((answer, _)) => answer.unwrap_or(T3Answer::Deny),
                // The terminal closed: the card goes with it.
                futures::future::Either::Right(_) => T3Answer::Deny,
            };
            if capability == T3Capability::Read && answer == T3Answer::UntilClosed {
                guard
                    .shared
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .shares
                    .entry((guard.extension_id.clone(), guard.terminal))
                    .or_default()
                    .insert(capability, T3Answer::UntilClosed);
            }
            answer
        }
        .boxed()
    }
}

/// The records in the consent file. A missing file means no consent yet; a broken one is logged
/// and treated the same, so the user is asked again rather than an extension let through.
/// The records and the first-use answers.
pub async fn read_file(path: &Path, fs: &dyn Fs) -> (BTreeMap<String, ConsentRecord>, Answers) {
    let Ok(text) = fs.load(path).await else {
        return Default::default();
    };
    if let Ok(read) = serde_json::from_str::<serde_json::Value>(&text) {
        BASE.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(path.to_path_buf())
            .or_insert(read);
    }
    match serde_json::from_str::<ConsentFile>(&text) {
        Ok(file) => (
            file.extensions,
            Answers {
                t2: file.t2,
                items: file.items,
                always_allow: file.always_allow,
            },
        ),
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
/// What this run last meant each consent file to hold: the file as it was read, then each save's
/// own contents. Another Fyx (Stable beside Dev or Preview: same data dir, separate instances)
/// writes the same file, so a save writes only this run's changes since then over what is on disk
/// now, instead of putting back what the other one revoked.
static BASE: std::sync::LazyLock<std::sync::Mutex<HashMap<PathBuf, serde_json::Value>>> =
    std::sync::LazyLock::new(Default::default);

/// Three-way merge of JSON documents: what `ours` changed since `base` goes over `theirs`. When
/// both changed the same value, a `false` wins over a `true` and a list keeps only what is in
/// both, so a revocation made by either side stays revoked; otherwise `ours` wins.
fn merge_json(
    base: Option<&serde_json::Value>,
    ours: Option<&serde_json::Value>,
    theirs: Option<&serde_json::Value>,
) -> Option<serde_json::Value> {
    use serde_json::Value;
    if ours == base {
        return theirs.cloned();
    }
    if theirs == base || theirs == ours {
        return ours.cloned();
    }
    match (base, ours, theirs) {
        (_, None, _) | (_, _, None) => None,
        (base, Some(Value::Object(ours)), Some(Value::Object(theirs))) => {
            let base = base.and_then(Value::as_object);
            let keys: BTreeSet<&String> = ours
                .keys()
                .chain(theirs.keys())
                .chain(base.into_iter().flat_map(|base| base.keys()))
                .collect();
            let merged = keys
                .into_iter()
                .filter_map(|key| {
                    merge_json(
                        base.and_then(|base| base.get(key)),
                        ours.get(key),
                        theirs.get(key),
                    )
                    .map(|value| (key.clone(), value))
                })
                .collect();
            Some(Value::Object(merged))
        }
        (_, Some(Value::Bool(ours)), Some(Value::Bool(theirs))) => {
            Some(Value::Bool(*ours && *theirs))
        }
        (_, Some(Value::Array(ours)), Some(Value::Array(theirs))) => Some(Value::Array(
            ours.iter()
                .filter(|value| theirs.contains(value))
                .cloned()
                .collect(),
        )),
        (_, Some(ours), Some(_)) => Some(ours.clone()),
    }
}

/// `text` (this run's newest contents of `path`) merged with what another Fyx wrote there since
/// this run last read or wrote it.
fn merged_with_disk(path: &Path, text: &str, on_disk: Option<&str>) -> String {
    let Ok(ours) = serde_json::from_str::<serde_json::Value>(text) else {
        return text.to_string();
    };
    let mut bases = BASE.lock().unwrap_or_else(PoisonError::into_inner);
    let theirs = on_disk.and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok());
    let merged = match &theirs {
        Some(theirs) => merge_json(bases.get(path), Some(&ours), Some(theirs)),
        None => Some(ours.clone()),
    };
    bases.insert(path.to_path_buf(), ours);
    merged
        .and_then(|merged| serde_json::to_string_pretty(&merged).ok())
        .unwrap_or_else(|| text.to_string())
}

/// Locks `<path>.lock` for this process until the file is dropped, so two Fyx instances do not
/// read and write `path` at once. `None` when it cannot be locked: the write goes ahead unlocked.
fn lock_beside(path: &Path) -> Option<std::fs::File> {
    let mut name = path.file_name()?.to_os_string();
    name.push(".lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path.with_file_name(name))
        .inspect_err(|error| log::warn!("fyx:ui: cannot lock {}: {error}", path.display()))
        .ok()?;
    match file.lock() {
        Ok(()) => Some(file),
        Err(error) => {
            log::warn!("fyx:ui: cannot lock {}: {error}", path.display());
            None
        }
    }
}

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
    // Held to the end of the save (the fake file system of tests has no file to lock).
    let _other_instances = (!fs.is_fake()).then(|| lock_beside(&path)).flatten();
    let on_disk = fs.load(&path).await.ok();
    let text = merged_with_disk(&path, &text, on_disk.as_deref());
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

    #[test]
    fn another_instances_revocation_survives_this_ones_save() {
        let path = PathBuf::from("/merge-test/fyx-ui-consent.json");
        let read = serde_json::json!({
            "extensions": {},
            "t2": { "a": { "network:fetch": true } },
            "items": { "a": { "items": { "fs:read#x": true }, "granted": [] } },
        });
        BASE.lock().unwrap().insert(path.clone(), read.clone());
        // This run grants something new to `b`; the other one revoked `a`'s grants meanwhile.
        let mut ours = read.clone();
        ours["t2"]["b"] = serde_json::json!({ "network:fetch": true });
        let mut theirs = read;
        theirs["t2"]["a"]["network:fetch"] = serde_json::json!(false);
        theirs["items"]["a"]["items"]["fs:read#x"] = serde_json::json!(false);
        let merged: serde_json::Value = serde_json::from_str(&merged_with_disk(
            &path,
            &ours.to_string(),
            Some(&theirs.to_string()),
        ))
        .unwrap();
        assert_eq!(merged["t2"]["a"]["network:fetch"], false);
        assert_eq!(merged["items"]["a"]["items"]["fs:read#x"], false);
        assert_eq!(merged["t2"]["b"]["network:fetch"], true);

        // Both changed the same answer: the revocation wins whichever side made it.
        let base = serde_json::json!({ "x": true, "caps": ["a", "b"] });
        let ours = serde_json::json!({ "x": false, "caps": ["a", "b", "c"] });
        let theirs = serde_json::json!({ "x": true, "caps": ["b"] });
        assert_eq!(
            merge_json(Some(&base), Some(&ours), Some(&theirs)),
            Some(serde_json::json!({ "x": false, "caps": ["b"] }))
        );
        // A key this run removed (an uninstall) stays removed; one only they added stays.
        let base = serde_json::json!({ "gone": 1 });
        let ours = serde_json::json!({});
        let theirs = serde_json::json!({ "gone": 1, "new": 2 });
        assert_eq!(
            merge_json(Some(&base), Some(&ours), Some(&theirs)),
            Some(serde_json::json!({ "new": 2 }))
        );
    }

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
                new_capabilities: vec!["network:fetch@*.cdn.example.org".into()],
                diff: ConsentDiff::default(),
            }
        );
        // A new T2 capability with a program, an argument pattern and an environment name.
        let v3 = manifest(
            "capabilities = [\"ui:panel\", \"network:fetch\", \"process:spawn\"]\n[network]\ndomains = [\"api.example.com\"]\n[[processes]]\nname = \"git\"\nargs = [\"status\"]\n[process]\nenv = [\"NO_COLOR\"]",
        );
        check(&v3).expect("valid");
        let ConsentState::NeedsApproval {
            new_capabilities, ..
        } = store.state("ext", &record(&v3))
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
                new_capabilities: vec!["workspace:diagnostics".into()],
                diff: ConsentDiff::default(),
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

    // --- 0.4: items, diffs, T3 (design wit-0.4 §C.2–§C.5) ---

    fn line(item: ConsentItem, tier: Tier, details: &[&str]) -> ItemLine {
        ItemLine {
            item,
            tier,
            title: String::new(),
            details: details.iter().map(|detail| detail.to_string()).collect(),
            reason: None,
            warnings: Vec::new(),
        }
    }

    fn program(name: &str) -> ConsentItem {
        ConsentItem::Program {
            capability: "process:spawn".into(),
            name: name.into(),
        }
    }

    fn capability(name: &str) -> ConsentItem {
        ConsentItem::Capability(name.into())
    }

    const DECLARED: &[&str] = &[
        "ui:panel",
        "context:editor",
        "process:spawn",
        "network:fetch",
        "terminal:read",
    ];

    fn lines_v1() -> Vec<ItemLine> {
        vec![
            line(capability("ui:panel"), Tier::T0, &[]),
            line(capability("context:editor"), Tier::T1, &[]),
            line(program("git"), Tier::T2, &["status"]),
            line(program("codex"), Tier::T2, &["exec *"]),
            line(capability("network:fetch"), Tier::T2, &["api.example.com"]),
            line(capability("terminal:read"), Tier::T3, &[]),
        ]
    }

    fn store_with_journal() -> (ConsentStore, Arc<JournalStore>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let journal = JournalStore::at(dir.path().join("journal"));
        let mut store = ConsentStore::at(dir.path().join("consent.json"));
        store.set_journal(journal.clone());
        store.declare("ext", DECLARED.iter().map(|c| c.to_string()).collect());
        (store, journal, dir)
    }

    fn consent_changes(journal: &JournalStore, extension_id: &str) -> Vec<(String, bool)> {
        journal
            .read(extension_id)
            .entries
            .into_iter()
            .filter_map(|entry| match entry.kind {
                JournalKind::ConsentChanged { item_key, allowed } => Some((item_key, allowed)),
                _ => None,
            })
            .collect()
    }

    /// §C.2: the install dialog's checkboxes. T1 is granted by installing, a ticked T2 item is
    /// granted and an unticked one is `Denied` (never `Ask`), T3 is asked each time. The page
    /// switches items, T1 too; a change streams the item, then its capability when that changed,
    /// and goes to the journal. The answers survive in the consent file.
    #[test]
    fn test_install_items() {
        use futures::{FutureExt as _, StreamExt as _};
        let (mut store, journal, dir) = store_with_journal();
        let checker = store.checker();
        let mut changes = checker.changes("ext");
        store
            .install(
                "ext",
                record(DECLARED, component()),
                lines_v1(),
                &[program("git")],
            )
            .expect("file contents");

        assert_eq!(
            checker.item_state("ext", &program("git")),
            PermissionState::Granted
        );
        assert_eq!(
            checker.item_state("ext", &program("codex")),
            PermissionState::Denied
        );
        assert_eq!(
            checker.state("ext", "process:spawn"),
            PermissionState::Granted
        );
        assert_eq!(
            checker.state("ext", "network:fetch"),
            PermissionState::Denied
        );
        assert_eq!(
            checker.request("ext", "network:fetch").now_or_never(),
            Some(PermissionState::Denied),
            "unticked is denied, not asked"
        );
        assert!(checker.check("ext", "network:fetch").is_err());
        assert!(checker.check("ext", "context:editor").is_ok());
        assert!(checker.check("ext", "ui:panel").is_ok());
        assert_eq!(checker.state("ext", "terminal:read"), PermissionState::Ask);
        assert!(checker.check("ext", "terminal:read").is_err());
        assert_eq!(
            store.items("ext"),
            vec![
                (capability("ui:panel"), PermissionState::Granted),
                (capability("context:editor"), PermissionState::Granted),
                (program("git"), PermissionState::Granted),
                (program("codex"), PermissionState::Denied),
                (capability("network:fetch"), PermissionState::Denied),
                (capability("terminal:read"), PermissionState::Ask),
            ]
        );
        let mut streamed = Vec::new();
        while let Some(Some(change)) = changes.next().now_or_never() {
            streamed.push(change);
        }
        assert!(
            streamed.contains(&("process:spawn".to_string(), PermissionState::Granted)),
            "{streamed:?}"
        );

        // The page turns git off: the item first, then its capability (now with no item on).
        store.set_item("ext", &program("git"), false).unwrap();
        assert_eq!(
            changes.next().now_or_never().flatten(),
            Some(("process:spawn#git".to_string(), PermissionState::Denied))
        );
        assert_eq!(
            changes.next().now_or_never().flatten(),
            Some(("process:spawn".to_string(), PermissionState::Denied))
        );
        assert!(changes.next().now_or_never().is_none());
        assert_eq!(
            checker.state("ext", "process:spawn"),
            PermissionState::Denied
        );
        // And codex on: the capability is back.
        store.set_item("ext", &program("codex"), true).unwrap();
        assert_eq!(
            checker.state("ext", "process:spawn"),
            PermissionState::Granted
        );
        assert!(
            store
                .revocable()
                .contains(&("ext".into(), "process:spawn".into()))
        );

        // T1 is revoked on the page too; T0 and T3 have no switch.
        store
            .set_item("ext", &capability("context:editor"), false)
            .unwrap();
        assert!(checker.check("ext", "context:editor").is_err());
        assert!(!store.is_granted("ext", "context:editor"));
        assert!(
            store
                .set_item("ext", &capability("ui:panel"), false)
                .is_err()
        );
        assert!(
            store
                .set_item("ext", &capability("terminal:read"), true)
                .is_err()
        );
        assert!(store.set_item("nobody", &program("git"), true).is_err());

        // The 0.3 revoke command turns off every item of the capability.
        store.revoke("ext", "process:spawn").unwrap();
        assert_eq!(
            checker.item_state("ext", &program("codex")),
            PermissionState::Denied
        );

        assert_eq!(
            consent_changes(&journal, "ext"),
            vec![
                ("process:spawn#git".to_string(), true),
                ("process:spawn#git".to_string(), false),
                ("process:spawn#codex".to_string(), true),
                ("context:editor".to_string(), false),
                ("process:spawn#codex".to_string(), false),
            ]
        );

        // The file round trip.
        let (_, text) = store
            .set_item("ext", &program("git"), true)
            .unwrap()
            .unwrap();
        let file: ConsentFile = serde_json::from_str(&text).unwrap();
        let mut loaded = ConsentStore::default();
        loaded.declare("ext", DECLARED.iter().map(|c| c.to_string()).collect());
        loaded.set_loaded(
            file.extensions,
            Answers {
                t2: file.t2,
                items: file.items,
                always_allow: file.always_allow,
            },
        );
        assert_eq!(loaded.items("ext"), store.items("ext"));
        assert_eq!(
            loaded.item_state("ext", &program("git")),
            PermissionState::Granted
        );
        drop(dir);
    }

    /// §C.5: an update is compared with what was consented to. A new T2 item or a new detail of
    /// one is held, with the diff; removals and T0 or T3 additions are not; widening an item
    /// that is `Denied` installs silently and it stays `Denied`. Approving installs with what is
    /// ticked: a widened item left unticked is `Denied`, an unchanged one keeps its answer.
    #[test]
    fn test_update_diff() {
        let (mut store, _journal, _dir) = store_with_journal();
        let v1 = lines_v1();
        let record_v1 = record(DECLARED, component());
        store.install(
            "ext",
            record_v1.clone(),
            v1.clone(),
            &[program("git"), capability("network:fetch")],
        );
        assert_eq!(
            store.state_with("ext", &record_v1, &v1),
            ConsentState::Allowed
        );

        // v2: git gains a pattern, codex (denied) gains one, "ls" is new, network:fetch goes, a
        // T0 surface is new.
        let v2 = vec![
            line(capability("ui:panel"), Tier::T0, &[]),
            line(capability("ui:status-item"), Tier::T0, &[]),
            line(capability("context:editor"), Tier::T1, &[]),
            line(program("git"), Tier::T2, &["status", "log *"]),
            line(program("codex"), Tier::T2, &["exec *", "resume"]),
            line(program("ls"), Tier::T2, &[]),
            line(capability("terminal:read"), Tier::T3, &[]),
        ];
        let diff = store.diff("ext", &v2);
        assert_eq!(
            diff.added
                .iter()
                .map(|l| l.item.clone())
                .collect::<Vec<_>>(),
            vec![program("ls")]
        );
        assert_eq!(diff.widened.len(), 1, "{diff:?}");
        assert_eq!(diff.widened[0].line.item, program("git"));
        assert_eq!(diff.widened[0].new_details, vec!["log *".to_string()]);
        assert_eq!(
            diff.removed
                .iter()
                .map(|l| l.item.clone())
                .collect::<Vec<_>>(),
            vec![capability("network:fetch")]
        );
        match store.state_with("ext", &record_v1, &v2) {
            ConsentState::NeedsApproval {
                new_capabilities,
                diff: held,
            } => {
                assert!(new_capabilities.is_empty());
                assert_eq!(held, diff);
            }
            other => panic!("{other:?}"),
        }

        // Approved with "ls" ticked and git not.
        store.install("ext", record_v1.clone(), v2.clone(), &[program("ls")]);
        assert_eq!(
            store.item_state("ext", &program("ls")),
            PermissionState::Granted
        );
        assert_eq!(
            store.item_state("ext", &program("git")),
            PermissionState::Denied
        );
        assert_eq!(
            store.item_state("ext", &program("codex")),
            PermissionState::Denied
        );
        assert_eq!(
            store.item_state("ext", &capability("context:editor")),
            PermissionState::Granted
        );
        assert_eq!(
            store.state_with("ext", &record_v1, &v2),
            ConsentState::Allowed
        );

        // A 0.3 record without lines: what it holds counts as consented, unchanged.
        let mut old = ConsentStore::default();
        old.grant("old", record(&["ui:panel", "context:editor"], component()));
        let old_lines = vec![
            line(capability("ui:panel"), Tier::T0, &[]),
            line(capability("context:editor"), Tier::T1, &[]),
        ];
        assert!(!old.diff("old", &old_lines).needs_approval());
        let mut more = old_lines;
        more.push(line(program("git"), Tier::T2, &[]));
        assert_eq!(old.diff("old", &more).added.len(), 1);
    }

    /// §C.3 "Always allow": only for a component with a hash, reset by a new one; it answers T3
    /// at once and is journaled.
    #[test]
    fn test_always_allow() {
        use futures::FutureExt as _;
        let (mut store, journal, _dir) = store_with_journal();
        let checker = store.checker();
        store.install("ext", record(DECLARED, component()), lines_v1(), &[]);
        assert!(store.can_always_allow("ext"));
        assert!(
            store
                .set_always_allow("ext", T3Capability::Write, true)
                .is_err(),
            "not declared"
        );
        store
            .set_always_allow("ext", T3Capability::Read, true)
            .unwrap();
        assert_eq!(
            store.always_allowed(),
            vec![("ext".to_string(), T3Capability::Read)]
        );
        assert_eq!(
            checker.state("ext", "terminal:read"),
            PermissionState::Granted
        );
        assert!(checker.check("ext", "terminal:read").is_ok());
        assert_eq!(
            checker
                .t3_request("ext", prompt(T3Capability::Read, 1))
                .now_or_never(),
            Some(T3Answer::Once)
        );

        // A new component: it no longer holds.
        store.grant(
            "ext",
            record(
                DECLARED,
                Grant::Component {
                    sha256: "def".into(),
                },
            ),
        );
        assert!(store.always_allowed().is_empty());
        assert_eq!(checker.state("ext", "terminal:read"), PermissionState::Ask);

        store
            .set_always_allow("ext", T3Capability::Read, true)
            .unwrap();
        store
            .set_always_allow("ext", T3Capability::Read, false)
            .unwrap();
        assert!(store.always_allowed().is_empty());
        assert_eq!(
            consent_changes(&journal, "ext"),
            vec![
                ("terminal:read#always".to_string(), true),
                ("terminal:read#always".to_string(), true),
                ("terminal:read#always".to_string(), false),
            ]
        );

        // A dev folder cannot have it.
        store.grant(
            "dev",
            record(
                DECLARED,
                Grant::DevFolder {
                    path: "C:/dev".into(),
                },
            ),
        );
        assert!(!store.can_always_allow("dev"));
        assert!(
            store
                .set_always_allow("dev", T3Capability::Read, true)
                .is_err()
        );
    }

    fn prompt(capability: T3Capability, terminal: HostTerminalId) -> T3Prompt {
        T3Prompt {
            capability,
            terminal,
            terminal_title: format!("Terminal {terminal}"),
            scope: None,
            text: None,
            submit: false,
        }
    }

    /// §C.3: T3 cards go to the prompter, one per terminal and four per extension (past them
    /// `Deny` with no card); a dropped answer or a missing prompter is `Deny`; a dropped request
    /// frees its place.
    #[test]
    fn test_t3_prompts() {
        use futures::{FutureExt as _, StreamExt as _};
        let mut store = ConsentStore::default();
        store.declare("ext", DECLARED.iter().map(|c| c.to_string()).collect());
        let checker = store.checker();
        assert_eq!(
            checker
                .t3_request("ext", prompt(T3Capability::Read, 1))
                .now_or_never(),
            Some(T3Answer::Deny),
            "no prompter"
        );
        let (prompter, mut cards) = mpsc::unbounded();
        store.set_t3_prompter(prompter);
        assert_eq!(
            checker
                .t3_request("ext", prompt(T3Capability::Write, 1))
                .now_or_never(),
            Some(T3Answer::Deny),
            "terminal:write is not declared"
        );

        let mut first = checker.t3_request("ext", prompt(T3Capability::Read, 1));
        let (extension_id, card, answer) = cards.next().now_or_never().flatten().unwrap();
        assert_eq!(extension_id, "ext");
        assert_eq!(card, prompt(T3Capability::Read, 1));
        assert_eq!(
            checker
                .t3_request("ext", prompt(T3Capability::Read, 1))
                .now_or_never(),
            Some(T3Answer::Deny),
            "one card per terminal"
        );
        assert!(cards.next().now_or_never().is_none());
        assert!((&mut first).now_or_never().is_none());
        answer.send(T3Answer::Once).unwrap();
        assert_eq!(first.now_or_never(), Some(T3Answer::Once));

        // Four terminals at once; the fifth is refused; a dropped request frees its place.
        let mut open: Vec<_> = (1..=4)
            .map(|terminal| checker.t3_request("ext", prompt(T3Capability::Read, terminal)))
            .collect();
        let mut answers: Vec<_> = (0..4)
            .map(|_| cards.next().now_or_never().flatten().unwrap().2)
            .collect();
        assert_eq!(
            checker
                .t3_request("ext", prompt(T3Capability::Read, 5))
                .now_or_never(),
            Some(T3Answer::Deny)
        );
        drop(open.pop());
        assert!(answers.pop().unwrap().is_canceled(), "the card sees it");
        let fifth = checker.t3_request("ext", prompt(T3Capability::Read, 5));
        assert!(cards.next().now_or_never().flatten().is_some());
        // A card closed without an answer is `Deny`.
        drop(answers);
        for request in open {
            assert_eq!(request.now_or_never(), Some(T3Answer::Deny));
        }
        drop(fifth);
    }

    /// §C.3: a terminal shared for reading needs no card (once, or until it closes); a card
    /// answered "until it closes" shares it; writes need a terminal shared for writing and still
    /// show their card; a closed terminal drops its shares and its open cards.
    #[test]
    fn test_shares() {
        use futures::{FutureExt as _, StreamExt as _};
        let mut store = ConsentStore::default();
        let mut declared: Vec<String> = DECLARED.iter().map(|c| c.to_string()).collect();
        declared.push("terminal:write".into());
        store.declare("ext", declared);
        let checker = store.checker();
        let (prompter, mut cards) = mpsc::unbounded();
        store.set_t3_prompter(prompter);
        let ask = |capability, terminal| checker.t3_request("ext", prompt(capability, terminal));

        store.share_terminal("ext", 7, T3Capability::Read, T3Answer::Once);
        assert_eq!(
            store.shared(7),
            vec![("ext".to_string(), T3Capability::Read)]
        );
        assert_eq!(
            ask(T3Capability::Read, 7).now_or_never(),
            Some(T3Answer::Once)
        );
        assert!(store.shared(7).is_empty(), "once is used up");
        assert!(cards.next().now_or_never().is_none());

        // A card answered "until it closes".
        let read = ask(T3Capability::Read, 7);
        let (_, _, answer) = cards.next().now_or_never().flatten().unwrap();
        answer.send(T3Answer::UntilClosed).unwrap();
        assert_eq!(read.now_or_never(), Some(T3Answer::UntilClosed));
        assert_eq!(
            ask(T3Capability::Read, 7).now_or_never(),
            Some(T3Answer::Once)
        );
        assert_eq!(
            ask(T3Capability::Read, 7).now_or_never(),
            Some(T3Answer::Once)
        );

        // Writes: refused with no card until shared, then each one on its card.
        assert_eq!(
            ask(T3Capability::Write, 7).now_or_never(),
            Some(T3Answer::Deny)
        );
        assert!(cards.next().now_or_never().is_none());
        store.share_terminal("ext", 7, T3Capability::Write, T3Answer::UntilClosed);
        let mut write = ask(T3Capability::Write, 7);
        assert!((&mut write).now_or_never().is_none());
        let (_, card, _answer) = cards.next().now_or_never().flatten().unwrap();
        assert_eq!(card.capability, T3Capability::Write);

        // The terminal closes: the open card ends with Deny, and nothing stays shared.
        store.terminal_closed(7);
        assert_eq!(write.now_or_never(), Some(T3Answer::Deny));
        assert!(store.shared(7).is_empty());
        assert_eq!(
            ask(T3Capability::Write, 7).now_or_never(),
            Some(T3Answer::Deny)
        );

        store.share_terminal("ext", 8, T3Capability::Write, T3Answer::UntilClosed);
        store.unshare("ext", 8);
        assert!(store.shared(8).is_empty());
    }

    /// §C.4: an uninstall forgets the items and "Always allow" and clears the journal.
    #[test]
    fn test_forget_items_and_journal() {
        let (mut store, journal, _dir) = store_with_journal();
        let checker = store.checker();
        store.install(
            "ext",
            record(DECLARED, component()),
            lines_v1(),
            &[program("git")],
        );
        store
            .set_always_allow("ext", T3Capability::Read, true)
            .unwrap();
        assert!(!journal.read("ext").entries.is_empty());
        store.forget("ext");
        assert!(journal.read("ext").entries.is_empty());
        assert!(store.items("ext").is_empty());
        assert!(store.always_allowed().is_empty());
        assert_ne!(
            checker.item_state("ext", &program("git")),
            PermissionState::Granted
        );
    }
}
