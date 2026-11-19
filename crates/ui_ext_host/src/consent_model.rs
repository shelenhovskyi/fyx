//! The 0.4 consent model (docs/fyx/wit-0.4/design.md §C.1–§C.5), as agreed with W2c on
//! 2026-11-19 (W2c's `tasks/progress/W2c-broker-api.md`): consent items, the dialog's lines and
//! the update diff.
//!
//! **A stand-in.** W2c's `ui_ext_wasm::permissions` (`ConsentItem`, `ItemLine`,
//! `consent_items`, `consent_diff`) and `ConsentStore`'s item API (`install`, `set_item`,
//! `item_state`) replace this module when they land; the names and shapes are the same, so the
//! consent UI only changes its imports. Until then [`ConsentBroker`] maps items onto the 0.3 store,
//! one T2 answer per capability, and it fails closed: a capability is allowed only when every one
//! of its items is ticked.

use std::path::PathBuf;

use ui_ext_wasm::{
    UiManifest,
    permissions::{PermissionState, Tier, tier_of},
};

use crate::consent::{ConsentRecord, ConsentStore};

/// §C.1, with T3 (W2c adds `Tier::T3` to `ui_ext_wasm::permissions`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ConsentTier {
    T0,
    T1,
    T2,
    T3,
}

pub const T3_CAPABILITIES: [&str; 2] = ["terminal:read", "terminal:write"];

pub fn consent_tier(capability: &str) -> Option<ConsentTier> {
    if T3_CAPABILITIES.contains(&capability) {
        return Some(ConsentTier::T3);
    }
    tier_of(capability).map(|tier| match tier {
        Tier::T0 => ConsentTier::T0,
        Tier::T1 => ConsentTier::T1,
        Tier::T2 => ConsentTier::T2,
    })
}

/// One thing the user agrees to (§C.2): a capability, or one program, listener or discovery
/// file of it, or the stronger `scope = "all-new"` of `terminal:env`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ConsentItem {
    Capability(String),
    /// `process:spawn` and `terminal:create`: one per declared program.
    Program {
        capability: String,
        name: String,
    },
    Listener(String),
    DiscoveryFile(String),
    EnvAllNew,
}

impl ConsentItem {
    pub fn capability(&self) -> &str {
        match self {
            ConsentItem::Capability(capability) | ConsentItem::Program { capability, .. } => {
                capability
            }
            ConsentItem::Listener(_) => ui_ext::integration::CAPABILITY_NETWORK_LISTEN,
            ConsentItem::DiscoveryFile(_) => ui_ext::integration::CAPABILITY_DISCOVERY_FILE,
            ConsentItem::EnvAllNew => "terminal:env",
        }
    }

    /// Stable across versions: the consent file and the page use it.
    pub fn key(&self) -> String {
        match self {
            ConsentItem::Capability(capability) => capability.clone(),
            ConsentItem::Program { capability, name } => format!("{capability}#{name}"),
            ConsentItem::Listener(name) | ConsentItem::DiscoveryFile(name) => {
                format!("{}#{name}", self.capability())
            }
            ConsentItem::EnvAllNew => "terminal:env#all-new".into(),
        }
    }
}

/// A line of the install dialog, the extension page or the update diff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemLine {
    pub item: ConsentItem,
    pub tier: ConsentTier,
    pub title: String,
    /// Argument patterns, domains, `NAME=value`, channels: what "widened" compares.
    pub details: Vec<String>,
    /// The manifest's `reason`, drawn as "The author says: …".
    pub reason: Option<String>,
    /// §C.1's combinations and the env notes, drawn as given.
    pub warnings: Vec<String>,
}

impl ItemLine {
    fn new(item: ConsentItem, tier: ConsentTier, title: impl Into<String>) -> Self {
        Self {
            item,
            tier,
            title: title.into(),
            details: Vec::new(),
            reason: None,
            warnings: Vec::new(),
        }
    }

    fn reason(mut self, reason: &str) -> Self {
        self.reason = Some(reason.trim().to_string()).filter(|reason| !reason.is_empty());
        self
    }

    /// Whether the user ticks it (T2); T0, T1 and T3 lines have no checkbox.
    pub fn has_checkbox(&self) -> bool {
        self.tier == ConsentTier::T2
    }
}

fn capability_title(capability: &str) -> String {
    match capability {
        "ui:panel" => "Adds panels to your docks".into(),
        "ui:status-bar" => "Adds items to the status bar".into(),
        "ui:modal" => "Opens dialogs after you click".into(),
        "ui:notifications" => "Shows notifications and progress".into(),
        "ui:editor-decorations" => "Adds highlights and notes to your editors".into(),
        "ui:images" => "Shows images".into(),
        "ui:canvas" => "Draws shapes and charts".into(),
        "ui:approvals" => "Shows a tool's approval questions as cards".into(),
        "package:files" => "Uses files from its own package".into(),
        "workspace:read-buffers" => "Reads your open files".into(),
        "workspace:read-files" => "Reads files in your projects".into(),
        "workspace:diagnostics" => "Reads errors and warnings".into(),
        "context:editor" => "Sees the file and selection you are editing".into(),
        "workspace:open" => "Opens files in the editor".into(),
        "terminal:profiles" => "Adds terminal profiles to the + menu".into(),
        "terminal:pseudo" => "Shows its own terminals".into(),
        "terminal:status" => "Shows a status on your terminals".into(),
        "review:propose" => "Proposes edits for you to review".into(),
        "network:fetch" => "Connects to the internet".into(),
        "terminal:env" => "Adds environment variables to its terminals".into(),
        "terminal:observe" => "Sees which terminals are open (titles, folders, status)".into(),
        "relay:hooks" => "Answers a tool's hooks through fyx-relay".into(),
        "workspace:edit" => "Edits files in your projects".into(),
        "agent:observe" => "Watches agent sessions".into(),
        "terminal:read" => "Reads a terminal you share with it".into(),
        "terminal:write" => "Types a command you confirm into a terminal you share".into(),
        other => format!("Unknown to this version of Fyx ({other})"),
    }
}

/// The dialog's groups 1–4 in order (§C.2): T0, then T1, then the T2 items, then T3.
pub fn consent_items(manifest: &UiManifest) -> Vec<ItemLine> {
    let mut lines = Vec::new();
    let has = |capability: &str| manifest.capabilities.iter().any(|c| c == capability);
    for capability in &manifest.capabilities {
        let Some(tier) = consent_tier(capability) else {
            continue;
        };
        let item = || ConsentItem::Capability(capability.clone());
        match capability.as_str() {
            "process:spawn" => {
                for process in &manifest.processes {
                    let mut line = ItemLine::new(
                        ConsentItem::Program {
                            capability: capability.clone(),
                            name: process.name.clone(),
                        },
                        tier,
                        format!("Runs {}", process.name),
                    );
                    line.details = process.args.clone();
                    if let Some(path) = &process.path {
                        line.details.insert(0, format!("at {path}"));
                    }
                    if ui_ext::process::runs_any_code(process) {
                        line.warnings
                            .push("It can run any program on your computer.".into());
                    }
                    lines.push(line);
                }
            }
            "terminal:create" => {
                for program in &manifest.terminal_programs {
                    let mut line = ItemLine::new(
                        ConsentItem::Program {
                            capability: capability.clone(),
                            name: program.name.clone(),
                        },
                        tier,
                        format!("Runs {} in a terminal of its own", program.name),
                    )
                    .reason(&program.reason);
                    line.details = program.args.clone();
                    if ui_ext::process::runs_any_code(&program.as_process()) {
                        line.warnings
                            .push("It can run any program on your computer.".into());
                    }
                    lines.push(line);
                }
            }
            "network:listen" => {
                for listener in &manifest.listeners {
                    lines.push(
                        ItemLine::new(
                            ConsentItem::Listener(listener.name.clone()),
                            tier,
                            format!(
                                "Listens on a local port for {} ({:?})",
                                listener.name, listener.protocol
                            ),
                        )
                        .reason(&listener.reason),
                    );
                }
            }
            "fs:discovery-file" => {
                for file in &manifest.discovery_files {
                    let mut line = ItemLine::new(
                        ConsentItem::DiscoveryFile(file.name.clone()),
                        tier,
                        format!("Writes a discovery file for {}", file.listener),
                    )
                    .reason(&file.reason);
                    line.details = vec![format!("{}/{}", file.dir, file.name)];
                    lines.push(line);
                }
            }
            "terminal:env" => {
                let Some(env) = &manifest.terminal_env else {
                    continue;
                };
                let mut line =
                    ItemLine::new(item(), tier, capability_title(capability)).reason(&env.reason);
                for (name, value) in &env.vars {
                    line.details.push(format!("{name}={value}"));
                    if value.contains("${package}") {
                        line.warnings
                            .push(format!("{name} points to files from this extension."));
                    }
                }
                lines.push(line);
                if env.scope == ui_ext::terminal::EnvScope::AllNew {
                    lines.push(
                        ItemLine::new(
                            ConsentItem::EnvAllNew,
                            tier,
                            "Also add these to every new terminal you open",
                        )
                        .reason(&env.reason),
                    );
                }
            }
            "network:fetch" => {
                let mut line = ItemLine::new(item(), tier, capability_title(capability));
                line.details = manifest
                    .network
                    .iter()
                    .flat_map(|network| network.domains.iter().cloned())
                    .collect();
                lines.push(line);
            }
            "relay:hooks" => {
                let mut line = ItemLine::new(item(), tier, capability_title(capability));
                line.details = manifest
                    .relay_channels
                    .iter()
                    .map(|channel| {
                        format!(
                            "{} (exits {} if Fyx does not answer)",
                            channel.name, channel.on_failure.exit
                        )
                    })
                    .collect();
                if let Some(channel) = manifest.relay_channels.first() {
                    line = line.reason(&channel.reason);
                }
                lines.push(line);
            }
            _ => lines.push(ItemLine::new(item(), tier, capability_title(capability))),
        }
    }
    // §C.1: a read with a way out says where it can go.
    const READS: [&str; 4] = [
        "context:editor",
        "workspace:read-buffers",
        "workspace:read-files",
        "terminal:read",
    ];
    let reads = READS.iter().any(|read| has(read));
    let out = has("network:fetch")
        || has("network:listen")
        || has("relay:hooks")
        || has("terminal:write")
        || has("terminal:create");
    lines.sort_by_key(|line| line.tier);
    if reads && out {
        let warning = "It can send what it reads outside Fyx (network, terminals or hooks).";
        if let Some(line) = lines
            .iter_mut()
            .find(|line| READS.contains(&line.item.capability()))
        {
            line.warnings.push(warning.into());
        }
    }
    lines
}

/// What an update changes against what the user agreed to (§C.5).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConsentDiff {
    pub added: Vec<ItemLine>,
    pub widened: Vec<ItemLine>,
    pub removed: Vec<ItemLine>,
}

impl ConsentDiff {
    /// Removals and T0 additions install silently.
    pub fn needs_approval(&self) -> bool {
        !self.added.is_empty() || !self.widened.is_empty()
    }
}

/// `granted`: the lines the user agreed to (stored in the record); `new`: the update's. An item
/// the user denied is neither added nor widened: it stays denied.
pub fn consent_diff(granted: &[ItemLine], denied: &[ConsentItem], new: &[ItemLine]) -> ConsentDiff {
    let mut diff = ConsentDiff::default();
    for line in new.iter().filter(|line| line.tier != ConsentTier::T0) {
        if denied.contains(&line.item) {
            continue;
        }
        match granted.iter().find(|old| old.item == line.item) {
            None => diff.added.push(line.clone()),
            Some(old)
                if line
                    .details
                    .iter()
                    .any(|detail| !old.details.contains(detail)) =>
            {
                diff.widened.push(line.clone())
            }
            Some(_) => {}
        }
    }
    diff.removed = granted
        .iter()
        .filter(|old| !new.iter().any(|line| line.item == old.item))
        .cloned()
        .collect();
    diff
}

/// The broker side the consent UI calls (W2c's `ConsentStore` item API).
pub trait ConsentBroker {
    /// The install dialog's "Install": records consent; unticked T2 items are denied.
    fn install(
        &mut self,
        extension_id: &str,
        record: ConsentRecord,
        lines: &[ItemLine],
        ticked: &[ConsentItem],
    ) -> Option<(PathBuf, String)>;
    /// The extension page's switch.
    fn set_item(
        &mut self,
        extension_id: &str,
        lines: &[ItemLine],
        item: &ConsentItem,
        allowed: bool,
    ) -> Option<(PathBuf, String)>;
    fn item_state(&self, extension_id: &str, item: &ConsentItem) -> PermissionState;
}

/// The stand-in over the 0.3 store: one T2 answer per capability, allowed only when all its
/// items are.
impl ConsentBroker for ConsentStore {
    fn install(
        &mut self,
        extension_id: &str,
        record: ConsentRecord,
        lines: &[ItemLine],
        ticked: &[ConsentItem],
    ) -> Option<(PathBuf, String)> {
        let mut file = self.grant(extension_id, record);
        let mut capabilities: Vec<&str> = lines
            .iter()
            .filter(|line| line.has_checkbox())
            .map(|line| line.item.capability())
            .collect();
        capabilities.dedup();
        for capability in capabilities {
            let allowed = lines
                .iter()
                .filter(|line| line.has_checkbox() && line.item.capability() == capability)
                .all(|line| ticked.contains(&line.item));
            file = self.set_t2(extension_id, capability, allowed).or(file);
        }
        file
    }

    fn set_item(
        &mut self,
        extension_id: &str,
        lines: &[ItemLine],
        item: &ConsentItem,
        allowed: bool,
    ) -> Option<(PathBuf, String)> {
        if !allowed {
            return self.set_t2(extension_id, item.capability(), false);
        }
        // Allowed once every item of the capability is: the stand-in has one answer for them.
        let others_allowed = lines
            .iter()
            .filter(|line| {
                line.has_checkbox()
                    && line.item.capability() == item.capability()
                    && &line.item != item
            })
            .all(|line| self.item_state(extension_id, &line.item) == PermissionState::Granted);
        if others_allowed {
            self.set_t2(extension_id, item.capability(), true)
        } else {
            None
        }
    }

    fn item_state(&self, extension_id: &str, item: &ConsentItem) -> PermissionState {
        match consent_tier(item.capability()) {
            Some(ConsentTier::T2) => self.t2_state(extension_id, item.capability()),
            Some(ConsentTier::T0) => PermissionState::Granted,
            Some(ConsentTier::T1) if self.is_granted(extension_id, item.capability()) => {
                PermissionState::Granted
            }
            Some(ConsentTier::T1) => PermissionState::Denied,
            Some(ConsentTier::T3) | None => PermissionState::Unavailable,
        }
    }
}
