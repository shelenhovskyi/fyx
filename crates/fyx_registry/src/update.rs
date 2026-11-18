//! The age gate and update planning (decisions 13, 17, 19, 21; spec §4 failures 1 and 2).
//!
//! [`plan_updates`] is level-triggered: it describes the state each installed extension should
//! be in given the current index and block list, so running it twice gives the same actions,
//! and an extension whose block is lifted simply stops getting a `Disable`. The host turns
//! changes between runs into notifications.

use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

use semver::Version;
use serde::{Deserialize, Serialize};

use crate::{
    BlockEntry, Error, ExtensionId, Release, Sha256Digest, Tier, Timestamp, VerifiedBlocklist,
    VerifiedIndex,
};

/// A release installs only once it has been in the index this long, unless it is `urgent`.
pub const AGE_GATE: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgeGate {
    Ready,
    /// Younger than the gate but flagged as a security fix.
    Urgent,
    Waiting {
        ready_at: Timestamp,
    },
}

impl AgeGate {
    pub fn allows_install(self) -> bool {
        matches!(self, AgeGate::Ready | AgeGate::Urgent)
    }
}

/// When this client first saw each release in an accepted index (integration-5 R8). The age gate
/// counts from the later of that and `published_at`: `published_at` is signed by the online
/// release key, so a leaked key could backdate a release to skip the wait, but it cannot change
/// when a client first saw it. A client with no history yet (the first index it ever accepts)
/// has nothing better than `published_at`, and records that.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FirstSeen(BTreeMap<String, Timestamp>);

impl FirstSeen {
    fn key(release: &Release) -> String {
        format!(
            "{}@{}#{}",
            release.id, release.version, release.package.sha256
        )
    }

    /// When the client first saw `release`, if it has.
    pub fn get(&self, release: &Release) -> Option<Timestamp> {
        self.0.get(&Self::key(release)).copied()
    }

    /// Records the releases of a newly accepted index: new ones at `now`, or at their
    /// `published_at` while the client has no history (`first_index`). Releases no longer in the
    /// index are forgotten, so this never outgrows the index.
    pub fn record(&mut self, index: &VerifiedIndex, now: Timestamp, first_index: bool) {
        let mut next = BTreeMap::new();
        for id in index.extensions() {
            for release in index.releases(id) {
                let key = Self::key(release);
                let seen = self.0.get(&key).copied().unwrap_or(if first_index {
                    release.published_at
                } else {
                    now
                });
                next.insert(key, seen);
            }
        }
        self.0 = next;
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A publish time in the future (a fast registry clock or a slow local one) counts as just
/// published: the wait is never shortened by skew. Without a first sight, `published_at` alone.
pub fn age_gate(release: &Release, now: Timestamp) -> AgeGate {
    age_gate_seen(release, None, now)
}

/// [`age_gate`] from the later of `published_at` and when the client first saw the release.
pub fn age_gate_seen(release: &Release, first_seen: Option<Timestamp>, now: Timestamp) -> AgeGate {
    let start = first_seen.map_or(release.published_at, |seen| seen.max(release.published_at));
    let ready_at = start.saturating_add(AGE_GATE);
    if now >= ready_at {
        AgeGate::Ready
    } else if release.urgent {
        AgeGate::Urgent
    } else {
        AgeGate::Waiting { ready_at }
    }
}

/// What the user has accepted for an installed extension: the active version's declaration
/// when it was installed, plus whatever a held update they approved asked for. Stored in
/// `state.json` by the install store.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Permissions {
    pub tier: Option<Tier>,
    pub capabilities: BTreeSet<String>,
    pub network_domains: BTreeSet<String>,
    pub processes: BTreeSet<String>,
}

impl Permissions {
    pub fn of(release: &Release) -> Self {
        Self {
            tier: Some(release.tier),
            capabilities: release.capabilities.clone(),
            network_domains: release.network_domains.clone(),
            processes: release.processes.clone(),
        }
    }

    /// What `release` asks for beyond these permissions that needs the user's approval. T0
    /// capabilities install silently; T1, T2 and unknown capabilities, new network domains,
    /// new programs and a higher tier do not (decision 21).
    pub fn additions(&self, release: &Release) -> Additions {
        let tier_raised = (release.tier > Tier::T0
            && self.tier.is_none_or(|approved| release.tier > approved))
        .then_some(release.tier);
        Additions {
            tier_raised,
            capabilities: release
                .capabilities
                .iter()
                .filter(|capability| {
                    Tier::of_capability_or_highest(capability) > Tier::T0
                        && !self.capabilities.contains(*capability)
                })
                .cloned()
                .collect(),
            network_domains: release
                .network_domains
                .difference(&self.network_domains)
                .cloned()
                .collect(),
            processes: release
                .processes
                .difference(&self.processes)
                .cloned()
                .collect(),
        }
    }

    /// Records the approval of a held update.
    pub fn approve(&mut self, release: &Release) {
        self.tier = self.tier.max(Some(release.tier));
        self.capabilities
            .extend(release.capabilities.iter().cloned());
        self.network_domains
            .extend(release.network_domains.iter().cloned());
        self.processes.extend(release.processes.iter().cloned());
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Additions {
    pub tier_raised: Option<Tier>,
    pub capabilities: BTreeSet<String>,
    pub network_domains: BTreeSet<String>,
    pub processes: BTreeSet<String>,
}

impl Additions {
    pub fn is_empty(&self) -> bool {
        self.tier_raised.is_none()
            && self.capabilities.is_empty()
            && self.network_domains.is_empty()
            && self.processes.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// Installed from the registry; signed and auto-updated.
    Registry,
    /// A `.fyxext` file from disk: "Unsigned, not from the registry", never auto-updated.
    LocalFile,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledVersion {
    pub version: Version,
    pub origin: Origin,
    /// SHA-256 of the installed `.fyxext`, so hash block entries apply to it (security-2 S13).
    /// `None` for an install made before it was recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_sha256: Option<Sha256Digest>,
}

/// The version of a `.fyxext` from disk must be plain `major.minor.patch`, like a registry
/// release: a pre-release or build suffix could otherwise pick its way around a block entry
/// (security-2 S13). The install store calls this before installing a local file.
pub fn check_local_version(version: &Version) -> Result<(), Error> {
    if version.pre.is_empty() && version.build.is_empty() {
        Ok(())
    } else {
        Err(Error::Invalid(format!(
            "version {version} of a local package must be major.minor.patch without a suffix"
        )))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledExtension {
    pub id: ExtensionId,
    pub active: InstalledVersion,
    /// The version kept for rollback (decision 13).
    pub previous: Option<InstalledVersion>,
    /// The user pinned the active version: it is never changed automatically.
    pub pinned: bool,
    pub approved: Permissions,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Update {
        to: Version,
        reason: UpdateReason,
    },
    /// Switch to the kept previous version because the active one is blocked.
    Rollback {
        to: Version,
        block: BlockEntry,
    },
    /// The active version is blocked and nothing can replace it. The extension stays installed.
    Disable {
        block: BlockEntry,
    },
    /// A newer version exists but is not installed yet; the page shows why.
    Hold {
        version: Version,
        reasons: Vec<HoldReason>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateReason {
    Newer,
    /// The active version is blocked and this release is not.
    ReplacesBlocked(BlockEntry),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HoldReason {
    Pinned,
    TooNew {
        ready_at: Timestamp,
    },
    /// "Update needs approval".
    NeedsApproval(Additions),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedAction {
    pub id: ExtensionId,
    pub action: Action,
}

/// Actions for every installed extension, in the order given.
///
/// Per extension: if the active version is blocked, it is replaced by the newest release that
/// may install, else rolled back to the kept previous version if that is not blocked, else
/// disabled. A pinned extension is only ever disabled. Otherwise it updates to the newest
/// release that may install. Either way, if a newer release is held back, a `Hold` says why;
/// a held release is never installed, even to replace a blocked one (spec §4 failure 1).
pub fn plan_updates(
    installed: &[InstalledExtension],
    index: &VerifiedIndex,
    blocklist: Option<&VerifiedBlocklist>,
    first_seen: &FirstSeen,
    now: Timestamp,
) -> Vec<PlannedAction> {
    let mut plan = Vec::new();
    for extension in installed {
        for action in plan_extension(extension, index, blocklist, first_seen, now) {
            plan.push(PlannedAction {
                id: extension.id.clone(),
                action,
            });
        }
    }
    plan
}

fn plan_extension(
    extension: &InstalledExtension,
    index: &VerifiedIndex,
    blocklist: Option<&VerifiedBlocklist>,
    first_seen: &FirstSeen,
    now: Timestamp,
) -> Vec<Action> {
    let block_for = |version: &Version, package_sha256: Option<&Sha256Digest>| {
        blocklist.and_then(|blocklist| blocklist.block_for(&extension.id, version, package_sha256))
    };

    let mut update = None;
    let mut hold = None;
    // A file from disk never auto-updates (decision 18); the block list still applies to it.
    if extension.active.origin == Origin::Registry {
        for release in index.releases(&extension.id).rev() {
            if release.version <= extension.active.version {
                break;
            }
            if block_for(&release.version, Some(&release.package.sha256)).is_some() {
                continue;
            }
            let reasons = hold_reasons(extension, release, first_seen, now);
            if reasons.is_empty() {
                update = Some(release);
                break;
            }
            if hold.is_none() {
                hold = Some(Action::Hold {
                    version: release.version.clone(),
                    reasons,
                });
            }
        }
    }

    let mut actions = Vec::new();
    match block_for(
        &extension.active.version,
        extension.active.package_sha256.as_ref(),
    ) {
        Some(block) => {
            let rollback_target = extension.previous.as_ref().filter(|previous| {
                !extension.pinned
                    && block_for(&previous.version, previous.package_sha256.as_ref()).is_none()
            });
            actions.push(match (update, rollback_target) {
                (Some(release), _) => Action::Update {
                    to: release.version.clone(),
                    reason: UpdateReason::ReplacesBlocked(block.clone()),
                },
                (None, Some(previous)) => Action::Rollback {
                    to: previous.version.clone(),
                    block: block.clone(),
                },
                (None, None) => Action::Disable {
                    block: block.clone(),
                },
            });
        }
        None => {
            if let Some(release) = update {
                actions.push(Action::Update {
                    to: release.version.clone(),
                    reason: UpdateReason::Newer,
                });
            }
        }
    }
    actions.extend(hold);
    actions
}

fn hold_reasons(
    extension: &InstalledExtension,
    release: &Release,
    first_seen: &FirstSeen,
    now: Timestamp,
) -> Vec<HoldReason> {
    let mut reasons = Vec::new();
    if extension.pinned {
        reasons.push(HoldReason::Pinned);
    }
    if let AgeGate::Waiting { ready_at } = age_gate_seen(release, first_seen.get(release), now) {
        reasons.push(HoldReason::TooNew { ready_at });
    }
    let additions = extension.approved.additions(release);
    if !additions.is_empty() {
        reasons.push(HoldReason::NeedsApproval(additions));
    }
    reasons
}

/// How the plan changed between two runs of [`plan_updates`], for the host's notifications: a
/// new `Disable` or `Rollback` says "blocked: <reason>", a `Disable` that went away says the block
/// was lifted. The plan is level-triggered, so an unchanged plan gives no change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanChanges {
    /// In the new plan, not in the old one.
    pub added: Vec<PlannedAction>,
    /// In the old plan, not in the new one.
    pub removed: Vec<PlannedAction>,
}

impl PlanChanges {
    pub fn between(previous: &[PlannedAction], current: &[PlannedAction]) -> Self {
        Self {
            added: current
                .iter()
                .filter(|action| !previous.contains(action))
                .cloned()
                .collect(),
            removed: previous
                .iter()
                .filter(|action| !current.contains(action))
                .cloned()
                .collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallRefusal {
    NotInIndex,
    /// Every release of the extension is blocked.
    Blocked {
        block: BlockEntry,
    },
    /// No unblocked release is old enough yet; the newest one becomes installable at `ready_at`.
    TooNew {
        version: Version,
        ready_at: Timestamp,
    },
}

/// The release a fresh install from the registry picks: the newest one that is not blocked and
/// passes the age gate. The caller then asks for the T1 consent the release declares.
pub fn plan_install<'a>(
    id: &ExtensionId,
    index: &'a VerifiedIndex,
    blocklist: Option<&VerifiedBlocklist>,
    first_seen: &FirstSeen,
    now: Timestamp,
) -> Result<&'a Release, InstallRefusal> {
    let mut first_block = None;
    let mut newest_waiting = None;
    for release in index.releases(id).rev() {
        if let Some(block) = blocklist.and_then(|blocklist| {
            blocklist.block_for(id, &release.version, Some(&release.package.sha256))
        }) {
            first_block.get_or_insert(block);
            continue;
        }
        match age_gate_seen(release, first_seen.get(release), now) {
            AgeGate::Ready | AgeGate::Urgent => return Ok(release),
            AgeGate::Waiting { ready_at } => {
                newest_waiting.get_or_insert((release, ready_at));
            }
        }
    }
    if let Some((release, ready_at)) = newest_waiting {
        return Err(InstallRefusal::TooNew {
            version: release.version.clone(),
            ready_at,
        });
    }
    match first_block {
        Some(block) => Err(InstallRefusal::Blocked {
            block: block.clone(),
        }),
        None => Err(InstallRefusal::NotInIndex),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Index, Signed,
        blocklist::test_support::{blocklist, entry, verified},
        index::test_support::release,
        signing::test_keys::{keyset, registry_key},
    };

    const HOUR: u64 = 60 * 60;
    const PUBLISHED: Timestamp = Timestamp(1_000_000);
    const LATER: Timestamp = Timestamp(1_000_000 + 25 * HOUR);

    fn version(text: &str) -> Version {
        Version::parse(text).expect("valid version")
    }

    fn id() -> ExtensionId {
        ExtensionId::new("todo").expect("valid id")
    }

    fn index(releases: Vec<Release>) -> VerifiedIndex {
        let mut index = Index::new(Timestamp(0));
        index.releases = releases
            .into_iter()
            .map(|release| Signed::sign(release, &registry_key()))
            .collect();
        index.verify_strict(&keyset()).expect("verifies")
    }

    fn todo(version: &str) -> Release {
        release("todo", version)
    }

    fn t1(mut release: Release) -> Release {
        release.tier = Tier::T1;
        release.capabilities.insert("workspace:read-files".into());
        release
    }

    fn published(mut release: Release, at: Timestamp) -> Release {
        release.published_at = at;
        release
    }

    fn installed(active: &str, previous: Option<&str>) -> InstalledExtension {
        InstalledExtension {
            id: id(),
            active: InstalledVersion {
                version: version(active),
                origin: Origin::Registry,
                package_sha256: None,
            },
            previous: previous.map(|previous| InstalledVersion {
                version: version(previous),
                origin: Origin::Registry,
                package_sha256: None,
            }),
            pinned: false,
            approved: Permissions::of(&todo(active)),
        }
    }

    fn actions(
        extension: &InstalledExtension,
        index: &VerifiedIndex,
        blocklist: Option<&VerifiedBlocklist>,
        now: Timestamp,
    ) -> Vec<Action> {
        plan_updates(
            std::slice::from_ref(extension),
            index,
            blocklist,
            &FirstSeen::default(),
            now,
        )
        .into_iter()
        .map(|planned| {
            assert_eq!(planned.id, extension.id);
            planned.action
        })
        .collect()
    }

    fn update(to: &str) -> Action {
        Action::Update {
            to: version(to),
            reason: UpdateReason::Newer,
        }
    }

    #[test]
    fn age_gate_waits_24_hours_unless_urgent() {
        let release = todo("1.0.0");
        let ready_at = PUBLISHED.saturating_add(AGE_GATE);
        assert_eq!(age_gate(&release, PUBLISHED), AgeGate::Waiting { ready_at });
        assert_eq!(
            age_gate(&release, Timestamp(ready_at.0 - 1)),
            AgeGate::Waiting { ready_at }
        );
        assert_eq!(age_gate(&release, ready_at), AgeGate::Ready);
        assert_eq!(
            age_gate(&release, Timestamp(0)),
            AgeGate::Waiting { ready_at }
        );

        let mut urgent = release;
        urgent.urgent = true;
        assert_eq!(age_gate(&urgent, PUBLISHED), AgeGate::Urgent);
        assert!(AgeGate::Urgent.allows_install());
        assert_eq!(age_gate(&urgent, ready_at), AgeGate::Ready);
    }

    #[test]
    fn updates_to_the_newest_release_past_the_age_gate() {
        let index = index(vec![
            todo("1.0.0"),
            todo("1.1.0"),
            todo("1.2.0"),
            published(todo("1.3.0"), LATER),
        ]);
        assert_eq!(
            actions(&installed("1.0.0", None), &index, None, LATER),
            vec![
                update("1.2.0"),
                Action::Hold {
                    version: version("1.3.0"),
                    reasons: vec![HoldReason::TooNew {
                        ready_at: LATER.saturating_add(AGE_GATE)
                    }],
                },
            ]
        );
        assert_eq!(
            actions(
                &installed("1.2.0", None),
                &index,
                None,
                PUBLISHED.saturating_add(AGE_GATE)
            ),
            vec![Action::Hold {
                version: version("1.3.0"),
                reasons: vec![HoldReason::TooNew {
                    ready_at: LATER.saturating_add(AGE_GATE)
                }],
            },]
        );
        assert_eq!(
            actions(&installed("1.3.0", None), &index, None, LATER),
            vec![]
        );
    }

    /// integration-5 R8: a backdated `published_at` (a leaked release key) does not skip the
    /// wait; the gate counts from when this client first saw the release.
    #[test]
    fn age_gate_counts_from_first_sight() {
        let backdated = todo("1.1.0");
        let first_sight = LATER;
        let ready_at = first_sight.saturating_add(AGE_GATE);
        assert_eq!(age_gate(&backdated, LATER), AgeGate::Ready);
        assert_eq!(
            age_gate_seen(&backdated, Some(first_sight), LATER),
            AgeGate::Waiting { ready_at }
        );
        assert_eq!(
            age_gate_seen(&backdated, Some(first_sight), ready_at),
            AgeGate::Ready
        );
        // A first sight before `published_at` (clock skew) never shortens the wait.
        let fresh = published(todo("1.2.0"), LATER);
        assert_eq!(
            age_gate_seen(&fresh, Some(PUBLISHED), LATER),
            AgeGate::Waiting { ready_at }
        );

        let index = index(vec![todo("1.0.0"), backdated.clone()]);
        let mut seen = FirstSeen::default();
        seen.record(&index, first_sight, false);
        assert_eq!(seen.get(&backdated), Some(first_sight));
        let extension = installed("1.0.0", None);
        let plan = |seen: &FirstSeen, now| {
            plan_updates(std::slice::from_ref(&extension), &index, None, seen, now)
                .into_iter()
                .map(|planned| planned.action)
                .collect::<Vec<_>>()
        };
        assert!(
            !plan(&seen, LATER).contains(&update("1.1.0")),
            "{:?}",
            plan(&seen, LATER)
        );
        assert_eq!(plan(&seen, ready_at), vec![update("1.1.0")]);
        let install = plan_install(&id(), &index, None, &seen, LATER);
        assert!(
            !matches!(&install, Ok(release) if release.version == version("1.1.0")),
            "{install:?}"
        );
    }

    #[test]
    fn first_seen_keeps_old_sights_and_forgets_dropped_releases() {
        let old = todo("1.0.0");
        let new = published(todo("1.1.0"), LATER);
        let mut seen = FirstSeen::default();
        // The first index a client accepts has only `published_at` to go by.
        seen.record(&index(vec![old.clone()]), LATER, true);
        assert_eq!(seen.get(&old), Some(PUBLISHED));
        let later = LATER.saturating_add(std::time::Duration::from_secs(HOUR));
        seen.record(&index(vec![old.clone(), new.clone()]), later, false);
        assert_eq!(seen.get(&old), Some(PUBLISHED));
        assert_eq!(seen.get(&new), Some(later));
        // A re-signed package is a new release: same version, new hash.
        let mut repacked = new.clone();
        repacked.package.sha256 = crate::Sha256Digest([7; 32]);
        seen.record(
            &index(vec![repacked.clone()]),
            later.saturating_add(std::time::Duration::from_secs(HOUR)),
            false,
        );
        assert_eq!(seen.get(&old), None);
        assert_eq!(seen.get(&new), None);
        assert_eq!(
            seen.get(&repacked),
            Some(later.saturating_add(std::time::Duration::from_secs(HOUR)))
        );
    }

    #[test]
    fn urgent_releases_skip_the_age_gate() {
        let mut fix = published(todo("1.0.1"), LATER);
        fix.urgent = true;
        let index = index(vec![todo("1.0.0"), fix]);
        assert_eq!(
            actions(&installed("1.0.0", None), &index, None, LATER),
            vec![update("1.0.1")]
        );
    }

    #[test]
    fn new_t1_capability_is_held_and_t0_installs_silently() {
        let mut t0_addition = todo("1.1.0");
        t0_addition.capabilities.insert("ui:status-bar".into());
        let index = index(vec![todo("1.0.0"), t0_addition, t1(todo("1.2.0"))]);
        let extension = installed("1.0.0", None);
        assert_eq!(
            actions(&extension, &index, None, LATER),
            vec![
                update("1.1.0"),
                Action::Hold {
                    version: version("1.2.0"),
                    reasons: vec![HoldReason::NeedsApproval(Additions {
                        tier_raised: Some(Tier::T1),
                        capabilities: ["workspace:read-files".to_string()].into(),
                        ..Additions::default()
                    })],
                },
            ]
        );

        let mut approved = extension;
        approved.approved.approve(&t1(todo("1.2.0")));
        assert_eq!(
            actions(&approved, &index, None, LATER),
            vec![update("1.2.0")]
        );
    }

    #[test]
    fn new_domains_programs_and_unknown_capabilities_are_held() {
        let mut current = todo("1.0.0");
        current.tier = Tier::T2;
        current
            .capabilities
            .extend(["network:fetch".to_string(), "process:spawn".to_string()]);
        current.network_domains.insert("api.example.com".into());
        current.processes.insert("git".into());

        let mut next = current.clone();
        next.version = version("1.1.0");
        next.network_domains.insert("cdn.example.com".into());
        next.processes.insert("rg".into());
        next.capabilities.insert("ui:from-the-future".into());

        let index = index(vec![current.clone(), next]);
        let mut extension = installed("1.0.0", None);
        extension.approved = Permissions::of(&current);
        assert_eq!(
            actions(&extension, &index, None, LATER),
            vec![Action::Hold {
                version: version("1.1.0"),
                reasons: vec![HoldReason::NeedsApproval(Additions {
                    tier_raised: None,
                    capabilities: ["ui:from-the-future".to_string()].into(),
                    network_domains: ["cdn.example.com".to_string()].into(),
                    processes: ["rg".to_string()].into(),
                })],
            }]
        );
    }

    #[test]
    fn pinned_extensions_do_not_update() {
        let index = index(vec![todo("1.0.0"), todo("1.1.0")]);
        let mut extension = installed("1.0.0", None);
        extension.pinned = true;
        assert_eq!(
            actions(&extension, &index, None, LATER),
            vec![Action::Hold {
                version: version("1.1.0"),
                reasons: vec![HoldReason::Pinned],
            }]
        );
    }

    #[test]
    fn local_files_never_auto_update() {
        let index = index(vec![todo("1.0.0"), todo("1.1.0")]);
        let mut extension = installed("1.0.0", None);
        extension.active.origin = Origin::LocalFile;
        assert_eq!(actions(&extension, &index, None, LATER), vec![]);
    }

    #[test]
    fn blocked_releases_are_skipped_as_update_targets() {
        let index = index(vec![todo("1.0.0"), todo("1.1.0"), todo("1.2.0")]);
        let blocklist = verified(blocklist(1, vec![entry("todo", ">=1.2.0")]));
        assert_eq!(
            actions(&installed("1.0.0", None), &index, Some(&blocklist), LATER),
            vec![update("1.1.0")]
        );
    }

    #[test]
    fn blocked_version_is_replaced_by_an_installable_fix() {
        let mut fix = published(todo("1.2.1"), LATER);
        fix.urgent = true;
        let index = index(vec![todo("1.1.0"), todo("1.2.0"), fix]);
        let block = entry("todo", "=1.2.0");
        let blocklist = verified(blocklist(1, vec![block.clone()]));
        assert_eq!(
            actions(
                &installed("1.2.0", Some("1.1.0")),
                &index,
                Some(&blocklist),
                LATER
            ),
            vec![Action::Update {
                to: version("1.2.1"),
                reason: UpdateReason::ReplacesBlocked(block),
            }]
        );
    }

    #[test]
    fn blocked_version_rolls_back_to_the_kept_previous_one() {
        let index = index(vec![
            todo("1.1.0"),
            todo("1.2.0"),
            published(todo("1.2.1"), LATER),
        ]);
        let block = entry("todo", "=1.2.0");
        let blocklist = verified(blocklist(1, vec![block.clone()]));
        assert_eq!(
            actions(
                &installed("1.2.0", Some("1.1.0")),
                &index,
                Some(&blocklist),
                LATER
            ),
            vec![
                Action::Rollback {
                    to: version("1.1.0"),
                    block,
                },
                Action::Hold {
                    version: version("1.2.1"),
                    reasons: vec![HoldReason::TooNew {
                        ready_at: LATER.saturating_add(AGE_GATE)
                    }],
                },
            ]
        );
    }

    // Spec §4 failure 2: the only installed version is blocked and nothing else can install.
    #[test]
    fn blocked_version_without_a_fallback_is_disabled() {
        let index = index(vec![todo("1.1.0"), todo("1.2.0")]);
        let block = entry("todo", ">=1.1.0");
        let blocklist = verified(blocklist(1, vec![block.clone()]));
        let disabled = vec![Action::Disable { block }];
        assert_eq!(
            actions(&installed("1.2.0", None), &index, Some(&blocklist), LATER),
            disabled
        );
        assert_eq!(
            actions(
                &installed("1.2.0", Some("1.1.0")),
                &index,
                Some(&blocklist),
                LATER
            ),
            disabled
        );
        // Not in the index at all (a file from disk, or delisted): the block still applies.
        let mut local = installed("1.2.0", None);
        local.active.origin = Origin::LocalFile;
        assert_eq!(
            actions(&local, &VerifiedIndex::default(), Some(&blocklist), LATER),
            disabled
        );
    }

    // Spec §4 failure 1: an update with a new T2 capability is never approved, then the old
    // version is blocked. The extension is disabled, not upgraded to the held version.
    #[test]
    fn blocked_version_is_not_replaced_by_a_held_update() {
        let mut held = todo("1.1.0");
        held.tier = Tier::T2;
        held.capabilities.insert("network:fetch".into());
        held.network_domains.insert("example.com".into());
        let index = index(vec![todo("1.0.0"), held]);
        let block = entry("todo", "=1.0.0");
        let blocklist = verified(blocklist(1, vec![block.clone()]));
        assert_eq!(
            actions(&installed("1.0.0", None), &index, Some(&blocklist), LATER),
            vec![
                Action::Disable { block },
                Action::Hold {
                    version: version("1.1.0"),
                    reasons: vec![HoldReason::NeedsApproval(Additions {
                        tier_raised: Some(Tier::T2),
                        capabilities: ["network:fetch".to_string()].into(),
                        network_domains: ["example.com".to_string()].into(),
                        processes: BTreeSet::new(),
                    })],
                },
            ]
        );
    }

    #[test]
    fn pinned_blocked_version_is_disabled_not_moved() {
        let index = index(vec![todo("1.1.0"), todo("1.2.0"), todo("1.3.0")]);
        let block = entry("todo", "=1.2.0");
        let blocklist = verified(blocklist(1, vec![block.clone()]));
        let mut extension = installed("1.2.0", Some("1.1.0"));
        extension.pinned = true;
        assert_eq!(
            actions(&extension, &index, Some(&blocklist), LATER),
            vec![
                Action::Disable { block },
                Action::Hold {
                    version: version("1.3.0"),
                    reasons: vec![HoldReason::Pinned],
                },
            ]
        );
    }

    #[test]
    fn plans_cover_every_installed_extension_in_order() {
        let mut other = release("lint", "2.0.0");
        other.id = ExtensionId::new("lint").expect("valid");
        let mut other_next = other.clone();
        other_next.version = version("2.1.0");
        let index = index(vec![todo("1.0.0"), todo("1.1.0"), other, other_next]);
        let mut lint = installed("2.0.0", None);
        lint.id = ExtensionId::new("lint").expect("valid");
        let plan = plan_updates(
            &[lint, installed("1.0.0", None)],
            &index,
            None,
            &FirstSeen::default(),
            LATER,
        );
        assert_eq!(
            plan.iter()
                .map(|planned| (planned.id.to_string(), planned.action.clone()))
                .collect::<Vec<_>>(),
            vec![
                ("lint".to_string(), update("2.1.0")),
                ("todo".to_string(), update("1.1.0"))
            ]
        );
    }

    #[test]
    fn install_picks_the_newest_installable_release() {
        let index = index(vec![
            todo("1.0.0"),
            todo("1.1.0"),
            published(todo("1.2.0"), LATER),
        ]);
        let picked =
            plan_install(&id(), &index, None, &FirstSeen::default(), LATER).expect("installable");
        assert_eq!(picked.version, version("1.1.0"));

        let blocklist = verified(blocklist(1, vec![entry("todo", "=1.1.0")]));
        let picked = plan_install(
            &id(),
            &index,
            Some(&blocklist),
            &FirstSeen::default(),
            LATER,
        )
        .expect("installable");
        assert_eq!(picked.version, version("1.0.0"));
    }

    #[test]
    fn install_refusals_explain_themselves() {
        let unknown = ExtensionId::new("unknown").expect("valid");
        assert_eq!(
            plan_install(
                &unknown,
                &VerifiedIndex::default(),
                None,
                &FirstSeen::default(),
                LATER
            ),
            Err(InstallRefusal::NotInIndex)
        );

        let fresh = index(vec![published(todo("1.0.0"), LATER)]);
        assert_eq!(
            plan_install(&id(), &fresh, None, &FirstSeen::default(), LATER),
            Err(InstallRefusal::TooNew {
                version: version("1.0.0"),
                ready_at: LATER.saturating_add(AGE_GATE),
            })
        );

        let block = entry("todo", "*");
        let blocklist = verified(blocklist(1, vec![block.clone()]));
        let index = index(vec![todo("1.0.0")]);
        assert_eq!(
            plan_install(
                &id(),
                &index,
                Some(&blocklist),
                &FirstSeen::default(),
                LATER
            ),
            Err(InstallRefusal::Blocked { block })
        );
    }

    #[test]
    fn local_versions_are_plain() {
        assert!(check_local_version(&version("1.2.0")).is_ok());
        assert!(check_local_version(&version("1.2.0+local")).is_err());
        assert!(check_local_version(&version("1.2.0-rc.1")).is_err());
    }
}
