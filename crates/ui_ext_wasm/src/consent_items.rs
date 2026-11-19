//! The 0.4 consent model (docs/fyx/wit-0.4/design.md §C.1–§C.5), re-exported from
//! [`crate::permissions`]: what an install dialog, the extension page and an update's diff show,
//! one [`ItemLine`] per item. It works for 0.3 manifests too, which go through the same dialog
//! under this host (§C.2).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use ui_ext::{
    integration, process as model_process,
    terminal::{self, EnvScope, Template, TemplatePart, TemplateUse},
};

use crate::{
    UiManifest, edit, network, observe,
    permissions::{Tier, tier_of},
    process,
};

/// One thing the user consents to. Its [`ConsentItem::key`] is stable across versions, so a
/// stored grant finds its item again.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConsentItem {
    /// A whole capability: T0, T1, T3, and the T2 ones that have no items of their own.
    Capability(String),
    /// One program of `process:spawn` or `terminal:create`.
    Program { capability: String, name: String },
    /// One `[[fyx_ui.listeners]]` entry, by name.
    Listener(String),
    /// One `[[fyx_ui.discovery_files]]` entry, by name.
    DiscoveryFile(String),
    /// `terminal:env` with `scope = "all-new"`: the second, stronger checkbox (§B.4).
    EnvAllNew,
}

impl ConsentItem {
    pub fn capability(&self) -> &str {
        match self {
            ConsentItem::Capability(capability) => capability,
            ConsentItem::Program { capability, .. } => capability,
            ConsentItem::Listener(_) => integration::CAPABILITY_NETWORK_LISTEN,
            ConsentItem::DiscoveryFile(_) => integration::CAPABILITY_DISCOVERY_FILE,
            ConsentItem::EnvAllNew => terminal::CAPABILITY_ENV,
        }
    }

    /// `terminal:create#codex`, `network:listen#ide`, `terminal:env#all-new`, or the capability.
    pub fn key(&self) -> String {
        match self {
            ConsentItem::Capability(capability) => capability.clone(),
            ConsentItem::Program { capability, name } => format!("{capability}#{name}"),
            ConsentItem::Listener(name) => {
                format!("{}#{name}", integration::CAPABILITY_NETWORK_LISTEN)
            }
            ConsentItem::DiscoveryFile(name) => {
                format!("{}#{name}", integration::CAPABILITY_DISCOVERY_FILE)
            }
            ConsentItem::EnvAllNew => format!("{}#all-new", terminal::CAPABILITY_ENV),
        }
    }

    pub fn tier(&self) -> Option<Tier> {
        tier_of(self.capability())
    }
}

/// One line of the install dialog, the extension page or an update's diff. The text is the
/// host's; `reason` is the author's ("The author says: …"), plain text the manifest check passed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemLine {
    pub item: ConsentItem,
    pub tier: Tier,
    pub title: String,
    /// One fact each (a domain, an argument pattern, `NAME=value`): an update that adds one
    /// widens the item (§C.5).
    pub details: Vec<String>,
    pub reason: Option<String>,
    /// The §C.1 combinations and the §B.4 environment notes.
    pub warnings: Vec<String>,
}

/// What an update changes against what was granted (§C.5). Only T1 and T2 items are held: T0
/// and T3 additions install silently, and so do removals.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentDiff {
    pub added: Vec<ItemLine>,
    pub widened: Vec<Widened>,
    pub removed: Vec<ItemLine>,
}

/// An item the update widens: the new line, and the details it did not have.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Widened {
    pub line: ItemLine,
    pub new_details: Vec<String>,
}

impl ConsentDiff {
    /// Whether the update is held until the user approves it.
    pub fn needs_approval(&self) -> bool {
        !self.added.is_empty() || !self.widened.is_empty()
    }
}

/// `env` names a tool reads to load code from a directory: the line says the tool will load
/// this extension's plugins (§B.4). Data, compared in upper case.
pub const PLUGIN_ENV_NAMES: &[(&str, &str)] = &[("CLAUDE_CODE_PLUGIN_DIRS", "Claude Code")];

pub const WARNING_RUNS_ANY_CODE: &str = "It can run any program on your computer";

const READ_CAPABILITIES: &[&str] = &[
    "workspace:read-buffers",
    "workspace:read-files",
    "workspace:diagnostics",
    integration::CAPABILITY_CONTEXT_EDITOR,
];

fn title_of(capability: &str) -> String {
    match capability {
        "ui:panel" => "A panel",
        "ui:status-bar" => "Status bar items",
        "ui:modal" => "Dialogs",
        "ui:notifications" => "Notifications",
        "ui:editor-decorations" => "Marks in the editor",
        "ui:images" => "Images",
        "ui:canvas" => "Drawings",
        "ui:picker" => "Pickers",
        "ui:code-lens" => "Code lenses",
        "ui:hover" => "Hover cards",
        "ui:virtual-documents" => "Read-only documents",
        "ui:commands" => "Commands",
        "package:files" => "Files from its own package",
        "ui:approvals" => "Approval cards for its tools",
        "workspace:read-buffers" => "Reads your open files",
        "workspace:read-files" => "Reads files in your worktrees",
        "workspace:diagnostics" => "Reads diagnostics",
        "context:editor" => "Sees the active file and selection",
        "workspace:open" => "Opens files",
        "terminal:profiles" => "Adds terminal profiles",
        "terminal:pseudo" => "Opens terminals of its own",
        "terminal:status" => "Shows status on terminals",
        "review:propose" => "Proposes edits for your review",
        "network:fetch" => "Connects to the internet",
        "process:spawn" => "Runs programs",
        "workspace:edit" => "Edits files in your worktrees",
        "agent:observe" => "Watches the agent's threads",
        "network:listen" => "Listens on local ports",
        "terminal:create" => "Opens terminals that run programs",
        "terminal:env" => "Adds variables to its terminals",
        "terminal:observe" => "Sees every terminal's title, status and exit",
        "fs:discovery-file" => "Writes discovery files",
        "relay:hooks" => "Receives tools' hooks",
        "terminal:read" => "Reads a terminal you share with it",
        "terminal:write" => "Types into a terminal you share with it",
        other => other,
    }
    .to_string()
}

fn reason(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn program_details(spec: &model_process::ProcessSpec) -> Vec<String> {
    spec.path
        .iter()
        .map(|path| format!("from {path}"))
        .chain(spec.args.iter().map(|pattern| format!("with {pattern}")))
        .collect()
}

/// Groups 1–4 of the install dialog (§C.2): T0, T1, T2 items, then T3, each in manifest order.
pub fn consent_items(manifest: &UiManifest) -> Vec<ItemLine> {
    let declares = |capability: &str| manifest.capabilities.iter().any(|c| c == capability);
    let reads = READ_CAPABILITIES.iter().any(|c| declares(c));
    let sends_warning = |target: String| {
        reads
            .then(|| format!("It can send what it reads to {target}"))
            .into_iter()
            .collect::<Vec<_>>()
    };
    let mut lines = Vec::new();
    let line = |item: ConsentItem, title: String, details, reason, warnings| ItemLine {
        tier: item.tier().unwrap_or(Tier::T2),
        item,
        title,
        details,
        reason,
        warnings,
    };
    for capability in &manifest.capabilities {
        let capability = capability.as_str();
        let Some(tier) = tier_of(capability) else {
            continue;
        };
        if tier != Tier::T2 {
            continue;
        }
        let whole = |details: Vec<String>, reason: Option<String>, warnings: Vec<String>| {
            line(
                ConsentItem::Capability(capability.into()),
                title_of(capability),
                details,
                reason,
                warnings,
            )
        };
        match capability {
            network::CAPABILITY_NETWORK => {
                let domains: Vec<String> = manifest
                    .network
                    .iter()
                    .flat_map(|network| network.domains.iter().cloned())
                    .collect();
                let target = if domains.is_empty() {
                    "the internet".to_string()
                } else {
                    domains.join(", ")
                };
                lines.push(whole(domains, None, sends_warning(target)));
            }
            process::CAPABILITY_PROCESS if !manifest.processes.is_empty() => {
                for spec in &manifest.processes {
                    let warnings = model_process::runs_any_code(spec)
                        .then(|| WARNING_RUNS_ANY_CODE.to_string())
                        .into_iter()
                        .collect();
                    lines.push(line(
                        ConsentItem::Program {
                            capability: capability.into(),
                            name: spec.name.clone(),
                        },
                        format!("Runs {}", spec.name),
                        program_details(spec),
                        None,
                        warnings,
                    ));
                }
            }
            terminal::CAPABILITY_CREATE if !manifest.terminal_programs.is_empty() => {
                for spec in &manifest.terminal_programs {
                    let process = spec.as_process();
                    let warnings = model_process::runs_any_code(&process)
                        .then(|| WARNING_RUNS_ANY_CODE.to_string())
                        .into_iter()
                        .collect();
                    lines.push(line(
                        ConsentItem::Program {
                            capability: capability.into(),
                            name: spec.name.clone(),
                        },
                        format!("Opens terminals running {}", spec.name),
                        program_details(&process),
                        reason(&spec.reason),
                        warnings,
                    ));
                }
            }
            integration::CAPABILITY_NETWORK_LISTEN if !manifest.listeners.is_empty() => {
                for spec in &manifest.listeners {
                    let mut details = vec![format!("{:?}", spec.protocol).to_lowercase()];
                    if spec.token_in_env {
                        details.push("its token goes into its terminals".into());
                    }
                    lines.push(line(
                        ConsentItem::Listener(spec.name.clone()),
                        format!("Listens on a local port ({})", spec.name),
                        details,
                        reason(&spec.reason),
                        sends_warning("programs that connect to its listeners".into()),
                    ));
                }
            }
            integration::CAPABILITY_DISCOVERY_FILE if !manifest.discovery_files.is_empty() => {
                for spec in &manifest.discovery_files {
                    let mut details = vec![format!("{}/{}", spec.dir, spec.name)];
                    details.extend(spec.default_dir.iter().map(|dir| format!("or {dir}")));
                    details.push(format!("for listener {}", spec.listener));
                    lines.push(line(
                        ConsentItem::DiscoveryFile(spec.name.clone()),
                        format!("Writes the discovery file {}", spec.name),
                        details,
                        reason(&spec.reason),
                        Vec::new(),
                    ));
                }
            }
            terminal::CAPABILITY_ENV => {
                let (details, notes, reason_text, all_new) = match &manifest.terminal_env {
                    Some(env) => {
                        let (details, notes) = env_lines(env);
                        (
                            details,
                            notes,
                            reason(&env.reason),
                            env.scope == EnvScope::AllNew,
                        )
                    }
                    None => Default::default(),
                };
                lines.push(whole(details.clone(), reason_text.clone(), notes.clone()));
                if all_new {
                    lines.push(line(
                        ConsentItem::EnvAllNew,
                        "Also adds these to every new terminal you open".into(),
                        details,
                        reason_text,
                        notes,
                    ));
                }
            }
            integration::CAPABILITY_RELAY_HOOKS => {
                let details = manifest
                    .relay_channels
                    .iter()
                    .map(|channel| {
                        format!(
                            "channel {} (on failure: exit {})",
                            channel.name, channel.on_failure.exit
                        )
                    })
                    .collect();
                let reasons: Vec<&str> = manifest
                    .relay_channels
                    .iter()
                    .map(|channel| channel.reason.trim())
                    .filter(|reason| !reason.is_empty())
                    .collect();
                let reason_text = (!reasons.is_empty()).then(|| reasons.join(" "));
                lines.push(whole(
                    details,
                    reason_text,
                    sends_warning("the tools whose hooks it answers".into()),
                ));
            }
            edit::CAPABILITY_EDIT | observe::CAPABILITY_OBSERVE => {
                lines.push(whole(Vec::new(), None, Vec::new()))
            }
            _ => lines.push(whole(Vec::new(), None, Vec::new())),
        }
    }
    let simple = |tier: Tier| {
        manifest
            .capabilities
            .iter()
            .filter(move |capability| tier_of(capability) == Some(tier))
            .map(|capability| {
                let warnings = if capability == terminal::CAPABILITY_WRITE {
                    sends_warning("the terminals you share with it".into())
                } else {
                    Vec::new()
                };
                line(
                    ConsentItem::Capability(capability.clone()),
                    title_of(capability),
                    Vec::new(),
                    None,
                    warnings,
                )
            })
            .collect::<Vec<_>>()
    };
    let mut ordered = simple(Tier::T0);
    ordered.extend(simple(Tier::T1));
    ordered.extend(lines);
    ordered.extend(simple(Tier::T3));
    ordered
}

/// `NAME=value` per variable, and the notes: values in the package, and names that make a tool
/// load plugins (§B.4).
fn env_lines(env: &terminal::TerminalEnvSpec) -> (Vec<String>, Vec<String>) {
    let mut details = Vec::new();
    let mut notes = Vec::new();
    for (name, value) in &env.vars {
        details.push(format!("{name}={value}"));
        let in_package = Template::parse(value, TemplateUse::EnvValue, &|_| Some(true))
            .is_ok_and(|template| template.0.contains(&TemplatePart::Package));
        if in_package {
            notes.push(format!("{name}: files from this extension"));
        }
        let upper = name.to_ascii_uppercase();
        if let Some((_, tool)) = PLUGIN_ENV_NAMES.iter().find(|(known, _)| *known == upper) {
            notes.push(format!(
                "{name}: makes {tool} load this extension's plugins"
            ));
        }
    }
    (details, notes)
}

/// Every warning of the dialog once, in order: for a summary or `registry.py comment`.
pub fn consent_warnings(manifest: &UiManifest) -> Vec<String> {
    let mut seen = BTreeSet::new();
    consent_items(manifest)
        .into_iter()
        .flat_map(|line| line.warnings)
        .filter(|warning| seen.insert(warning.clone()))
        .collect()
}

/// An update against what was granted (the lines stored with the grant, not the old manifest).
/// Whether a widened item that is denied installs silently is the broker's call.
pub fn consent_diff(granted: &[ItemLine], new: &UiManifest) -> ConsentDiff {
    let new_lines = consent_items(new);
    let held = |line: &ItemLine| matches!(line.tier, Tier::T1 | Tier::T2);
    let mut diff = ConsentDiff::default();
    for line in &new_lines {
        match granted.iter().find(|old| old.item == line.item) {
            None if held(line) => diff.added.push(line.clone()),
            None => {}
            Some(old) => {
                let new_details: Vec<String> = line
                    .details
                    .iter()
                    .filter(|detail| !old.details.contains(detail))
                    .cloned()
                    .collect();
                if !new_details.is_empty() && held(line) {
                    diff.widened.push(Widened {
                        line: line.clone(),
                        new_details,
                    });
                }
            }
        }
    }
    diff.removed = granted
        .iter()
        .filter(|old| !new_lines.iter().any(|line| line.item == old.item))
        .cloned()
        .collect();
    diff
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(body: &str) -> UiManifest {
        // The `[fyx_ui]` table's own contents: `[fyx_ui.network]` becomes `[network]`.
        let body = body.replace("[fyx_ui.", "[");
        toml::from_str(&format!("component = \"fyx_ui.wasm\"\n{body}"))
            .unwrap_or_else(|error| panic!("{error}"))
    }

    const CODEX: &str = r#"
api_version = "0.4.0"
capabilities = ["ui:panel", "context:editor", "terminal:create", "terminal:env", "network:fetch", "terminal:read"]
[fyx_ui.network]
domains = ["api.example.com"]
[[fyx_ui.terminal_programs]]
name = "codex"
args = ["-c", "notify=*"]
reason = "runs Codex in a panel"
[[fyx_ui.terminal_programs]]
name = "bash"
reason = "a shell"
[fyx_ui.terminal_env]
vars = { CLAUDE_CODE_PLUGIN_DIRS = "${package}/plugin", MODE = "on" }
scope = "all-new"
reason = "connects the CLI"
"#;

    #[test]
    fn items_and_groups() {
        let lines = consent_items(&manifest(CODEX));
        let keys: Vec<String> = lines.iter().map(|line| line.item.key()).collect();
        assert_eq!(
            keys,
            [
                "ui:panel",
                "context:editor",
                "terminal:create#codex",
                "terminal:create#bash",
                "terminal:env",
                "terminal:env#all-new",
                "network:fetch",
                "terminal:read",
            ]
        );
        let tiers: Vec<Tier> = lines.iter().map(|line| line.tier).collect();
        assert_eq!(
            tiers,
            [
                Tier::T0,
                Tier::T1,
                Tier::T2,
                Tier::T2,
                Tier::T2,
                Tier::T2,
                Tier::T2,
                Tier::T3
            ]
        );
        let codex = &lines[2];
        assert_eq!(codex.details, ["with -c", "with notify=*"]);
        assert_eq!(codex.reason.as_deref(), Some("runs Codex in a panel"));
        assert!(codex.warnings.is_empty());
        assert_eq!(lines[3].warnings, [WARNING_RUNS_ANY_CODE]);
        let env = &lines[4];
        assert_eq!(
            env.details,
            ["CLAUDE_CODE_PLUGIN_DIRS=${package}/plugin", "MODE=on"]
        );
        assert_eq!(
            env.warnings,
            [
                "CLAUDE_CODE_PLUGIN_DIRS: files from this extension",
                "CLAUDE_CODE_PLUGIN_DIRS: makes Claude Code load this extension's plugins",
            ]
        );
        // A read with the network: the combination is spelled out.
        assert_eq!(
            lines[6].warnings,
            ["It can send what it reads to api.example.com"]
        );
        assert_eq!(consent_warnings(&manifest(CODEX)).len(), 4);
        // Stable keys round-trip through the record.
        let json = toml::to_string(&lines[2]).unwrap();
        assert_eq!(toml::from_str::<ItemLine>(&json).unwrap(), lines[2]);
    }

    #[test]
    fn a_0_3_manifest() {
        let lines = consent_items(&manifest(
            r#"
api_version = "0.3.0"
capabilities = ["ui:panel", "process:spawn", "workspace:edit"]
[[fyx_ui.processes]]
name = "git"
args = ["status"]
"#,
        ));
        let keys: Vec<String> = lines.iter().map(|line| line.item.key()).collect();
        assert_eq!(keys, ["ui:panel", "process:spawn#git", "workspace:edit"]);
        assert!(lines[2].warnings.is_empty());
    }

    #[test]
    fn diffs() {
        let granted = consent_items(&manifest(CODEX));
        assert!(!consent_diff(&granted, &manifest(CODEX)).needs_approval());
        let wider = CODEX
            .replace("\"notify=*\"]", "\"notify=*\", \"**\"]")
            .replace(
                "\"ui:panel\",",
                "\"ui:panel\", \"ui:notifications\", \"workspace:read-files\",",
            )
            .replace("\"network:fetch\", ", "")
            .replace("domains = [\"api.example.com\"]", "domains = []");
        let diff = consent_diff(&granted, &manifest(&wider));
        // A new T1 item is held, a new T0 one is not.
        let added: Vec<String> = diff.added.iter().map(|line| line.item.key()).collect();
        assert_eq!(added, ["workspace:read-files"]);
        let widened: Vec<(String, Vec<String>)> = diff
            .widened
            .iter()
            .map(|widened| (widened.line.item.key(), widened.new_details.clone()))
            .collect();
        assert_eq!(
            widened,
            [(
                "terminal:create#codex".to_string(),
                vec!["with **".to_string()]
            )]
        );
        let removed: Vec<String> = diff.removed.iter().map(|line| line.item.key()).collect();
        assert_eq!(removed, ["network:fetch"]);
        assert!(diff.needs_approval());
        // Narrowing and removals install silently.
        let narrower = CODEX.replace("args = [\"-c\", \"notify=*\"]", "args = [\"-c\"]");
        assert!(!consent_diff(&granted, &manifest(&narrower)).needs_approval());
    }
}
