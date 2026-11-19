//! Tests of the registry service (platform plan 3.3–3.5) against a local test registry: a
//! directory with `snapshot.json`, `index.json`, `blocklist.json` and packages, signed with the
//! test keys. The private seeds below exist only here.

use std::{collections::BTreeSet, path::Path, sync::Arc, time::Duration};

use fyx_registry::{
    Blocklist, ExtensionId, Index, IndexRefresh, KeyId, KeyRole, Keyset, Origin, Package,
    Permissions, Release, Sha256Digest, Signed, SigningKey, Snapshot, Source, Tier, Timestamp,
};
use semver::Version;

use crate::registry_service::{
    Registry, RegistryLocation, RegistryStateDir, TEST_KEYSET_JSON, compiled_keyset, now,
    release_keyset,
};

const RELEASE_KEY: (&str, [u8; 32]) = ("fyx-test-release", [0x11; 32]);
const BLOCKLIST_KEY: (&str, [u8; 32]) = ("fyx-test-blocklist", [0x22; 32]);
const SNAPSHOT_KEY: (&str, [u8; 32]) = ("fyx-test-snapshot", [0x33; 32]);

fn signing_key((id, seed): (&str, [u8; 32])) -> SigningKey {
    SigningKey::from_seed(KeyId::new(id).expect("key id"), &seed).expect("seed")
}

fn test_keyset_from_seeds() -> Keyset {
    Keyset::new(vec![
        signing_key(RELEASE_KEY).public_key([KeyRole::Release]),
        signing_key(BLOCKLIST_KEY).public_key([KeyRole::Blocklist]),
        signing_key(SNAPSHOT_KEY).public_key([KeyRole::Snapshot]),
    ])
    .expect("keyset")
}

/// The compiled-in test keyset is exactly the public half of the seeds here.
#[test]
fn test_compiled_test_keyset_matches_the_seeds() {
    let expected = test_keyset_from_seeds();
    let json = serde_json::to_string_pretty(&expected).expect("json");
    assert_eq!(
        Keyset::from_json(TEST_KEYSET_JSON.as_bytes()).ok().as_ref(),
        Some(&expected),
        "fixtures/registry/test-keyset.json should be:\n{json}"
    );
    assert_eq!(
        compiled_keyset(),
        Some(expected),
        "debug and test builds trust it"
    );
}

/// Coordinator rule (c): a release build never trusts a test key.
#[test]
fn test_release_keyset_has_no_test_keys() {
    let test_ids: BTreeSet<String> = test_keyset_from_seeds()
        .keys()
        .iter()
        .map(|key| key.key_id.to_string())
        .collect();
    if let Some(release) = release_keyset() {
        for key in release.keys() {
            assert!(
                !test_ids.contains(&key.key_id.to_string())
                    && !key.key_id.to_string().contains("test"),
                "the release keyset contains the test key {}",
                key.key_id
            );
        }
    }
}

#[test]
fn test_registry_locations() {
    use RegistryLocation::*;
    assert_eq!(RegistryLocation::parse("").unwrap(), None);
    assert_eq!(
        RegistryLocation::parse("https://registry.example/fyx").unwrap(),
        Some(Http("https://registry.example/fyx/".into()))
    );
    assert_eq!(
        RegistryLocation::parse("http://127.0.0.1:8080/").unwrap(),
        Some(Http("http://127.0.0.1:8080/".into()))
    );
    assert!(
        RegistryLocation::parse("http://registry.example/").is_err(),
        "https only"
    );
    assert!(RegistryLocation::parse("ftp://x/").is_err());
    assert!(matches!(
        RegistryLocation::parse("file:///tmp/registry").unwrap(),
        Some(Directory(_))
    ));
}

/// A test registry on disk.
pub(crate) struct TestRegistry {
    pub dir: tempfile::TempDir,
    releases: Vec<Signed<Release>>,
    sequence: u64,
}

impl TestRegistry {
    pub fn new() -> Self {
        Self {
            dir: tempfile::tempdir().expect("temp dir"),
            releases: Vec::new(),
            sequence: 0,
        }
    }

    pub fn location(&self) -> RegistryLocation {
        RegistryLocation::Directory(self.dir.path().to_path_buf())
    }

    /// Publishes `package` as `id` `version` (2 days old, past the age gate).
    pub fn publish(&mut self, id: &str, version: &str, package: &[u8], capabilities: &[&str]) {
        self.publish_at(
            id,
            version,
            package,
            capabilities,
            Timestamp(now().0 - 2 * 24 * 60 * 60),
        );
    }

    /// [`Self::publish`] with its publication time.
    pub fn publish_at(
        &mut self,
        id: &str,
        version: &str,
        package: &[u8],
        capabilities: &[&str],
        published_at: Timestamp,
    ) {
        let path = format!("packages/{id}-{version}.fyxext");
        let file = self.dir.path().join(&path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, package).unwrap();
        let release = Release {
            id: ExtensionId::new(id).expect("id"),
            version: Version::parse(version).expect("version"),
            name: "Test Extension".into(),
            description: "An extension for tests.".into(),
            owners: ["fyx-tests".to_string()].into(),
            source: Source {
                repository: "https://github.com/fyx-tests/extension".into(),
                commit: "0123456789abcdef0123456789abcdef01234567".into(),
                path: ".".into(),
            },
            package: Package {
                url: format!("https://registry.test/{path}"),
                sha256: Sha256Digest::of(package),
                size: package.len() as u64,
            },
            published_at,
            urgent: false,
            tier: capabilities
                .iter()
                .map(|capability| Tier::of_capability_or_highest(capability))
                .max()
                .unwrap_or(Tier::T0),
            capabilities: capabilities
                .iter()
                .map(|capability| capability.to_string())
                .collect(),
            network_domains: BTreeSet::new(),
            processes: BTreeSet::new(),
            zed_extension: false,
        };
        self.releases
            .push(Signed::sign(release, &signing_key(RELEASE_KEY)));
        self.write_index();
    }

    /// Writes the index and a fresh snapshot naming it.
    pub fn write_index(&mut self) {
        let mut index = Index::new(now());
        index.releases = self.releases.clone();
        let index_bytes = index.to_json().expect("index json");
        std::fs::write(self.dir.path().join("index.json"), &index_bytes).unwrap();
        self.sequence += 1;
        let snapshot = Snapshot::for_index(
            &index_bytes,
            self.sequence,
            now(),
            Duration::from_secs(48 * 60 * 60),
            BTreeSet::new(),
        );
        let signed = Signed::sign(snapshot, &signing_key(SNAPSHOT_KEY));
        std::fs::write(
            self.dir.path().join("snapshot.json"),
            Snapshot::to_json(&signed).expect("snapshot json"),
        )
        .unwrap();
    }

    pub fn write_blocklist(&self, blocklist: Blocklist) {
        let signed = Signed::sign(blocklist, &signing_key(BLOCKLIST_KEY));
        std::fs::write(
            self.dir.path().join("blocklist.json"),
            serde_json::to_vec_pretty(&signed).expect("blocklist json"),
        )
        .unwrap();
    }

    pub fn package_path(&self, id: &str, version: &str) -> std::path::PathBuf {
        self.dir
            .path()
            .join(format!("packages/{id}-{version}.fyxext"))
    }
}

fn open(registry: &TestRegistry, state_dir: &Path) -> Registry {
    Registry::open(
        compiled_keyset(),
        registry.location(),
        Arc::new(http_client::BlockedHttpClient),
        RegistryStateDir(state_dir.to_path_buf()),
    )
    .expect("the test keyset is compiled in")
}

/// Plan 3.3: install from the index (the package is verified against the signed hash) into the
/// install store, recorded as a registry install with what was approved; a tampered package is
/// refused, and the accepted state survives a restart.
#[gpui::test]
async fn test_install_from_a_directory_registry(cx: &mut gpui::TestAppContext) {
    cx.executor().allow_parking();
    let mut test_registry = TestRegistry::new();
    let package = crate::install_store::test_support::package("demo", "1.0.0", b"component").await;
    test_registry.publish("demo", "1.0.0", &package, &["ui:status-bar"]);
    let state = tempfile::tempdir().unwrap();
    let registry = open(&test_registry, state.path());

    let refreshed = registry.refresh(now()).await;
    assert_eq!(refreshed.index, IndexRefresh::Updated);
    let id = ExtensionId::new("demo").unwrap();
    let (release, bytes) = registry
        .download_for_install(&id, now())
        .await
        .expect("installs from the index");
    assert_eq!(release.version, Version::new(1, 0, 0));
    assert_eq!(bytes, package);

    let store_dir = tempfile::tempdir().unwrap();
    let store = crate::install_store::InstallStore::new(store_dir.path().to_path_buf());
    let fs: Arc<dyn fs::Fs> = fs::RealFs::new(None, cx.executor());
    let source = crate::install_store::VersionSource {
        origin: Origin::Registry,
        package_sha256: Some(release.package.sha256),
    };
    store
        .install_package_bytes(
            bytes,
            "demo 1.0.0",
            source,
            Some(Permissions::of(&release)),
            fs.as_ref(),
        )
        .await
        .expect("unpacks");
    let installed = store
        .installed_extensions(fs.as_ref())
        .await
        .expect("state");
    assert_eq!(installed.len(), 1);
    assert_eq!(installed[0].active.origin, Origin::Registry);
    assert_eq!(
        installed[0].active.package_sha256,
        Some(release.package.sha256)
    );
    assert_eq!(installed[0].approved, Permissions::of(&release));

    // The state was stored: a new client knows the index without fetching.
    let reopened = open(&test_registry, state.path());
    assert!(reopened.client().lock().await.index(now()).is_ok());

    // A package changed after signing is refused.
    std::fs::write(test_registry.package_path("demo", "1.0.0"), b"tampered").unwrap();
    let error = registry
        .download_for_install(&id, now())
        .await
        .expect_err("tampered");
    assert!(format!("{error:#}").contains("does not match"), "{error:#}");
}

fn blocklist_blocking(id: &str, sequence: u64) -> Blocklist {
    Blocklist {
        schema: fyx_registry::BLOCKLIST_SCHEMA,
        sequence,
        generated_at: now(),
        expires_at: Timestamp(now().0 + 24 * 60 * 60),
        entries: vec![fyx_registry::BlockEntry {
            id: ExtensionId::new(id).unwrap(),
            versions: fyx_registry::VersionRange::all(),
            package_sha256: None,
            reason: "Steals tokens".into(),
            urgent: true,
        }],
    }
}

/// Plan 3.4: the block list is fetched with the index when due, cached, and acts on installed
/// versions (a block with nothing to roll back to disables); a block list signed with a release
/// key is refused and the cached one kept.
#[gpui::test]
async fn test_block_list_from_a_directory_registry(cx: &mut gpui::TestAppContext) {
    cx.executor().allow_parking();
    let mut test_registry = TestRegistry::new();
    let package = crate::install_store::test_support::package("demo", "1.0.0", b"component").await;
    test_registry.publish("demo", "1.0.0", &package, &["ui:status-bar"]);
    test_registry.write_blocklist(blocklist_blocking("demo", 1));
    let state = tempfile::tempdir().unwrap();
    let registry = open(&test_registry, state.path());
    let refreshed = registry.refresh(now()).await;
    assert!(
        matches!(
            refreshed.blocklist,
            Some(fyx_registry::CacheUpdate::Replace(_))
        ),
        "{:?}",
        refreshed.blocklist
    );
    let installed = [fyx_registry::InstalledExtension {
        id: ExtensionId::new("demo").unwrap(),
        active: fyx_registry::InstalledVersion {
            version: Version::new(1, 0, 0),
            origin: Origin::Registry,
            package_sha256: Some(Sha256Digest::of(&package)),
        },
        previous: None,
        pinned: false,
        approved: Permissions::default(),
    }];
    let actions = registry
        .client()
        .lock()
        .await
        .plan_updates(&installed, now());
    assert!(
        matches!(
            actions.as_slice(),
            [fyx_registry::PlannedAction {
                action: fyx_registry::Action::Disable { .. },
                ..
            }]
        ),
        "{actions:?}"
    );
    // Not due again for 6 h: the next refresh does not ask.
    assert_eq!(registry.refresh(now()).await.blocklist, None);

    // A list signed with the release key, on a fresh client: refused.
    let forged = Signed::sign(blocklist_blocking("other", 2), &signing_key(RELEASE_KEY));
    std::fs::write(
        test_registry.dir.path().join("blocklist.json"),
        serde_json::to_vec_pretty(&forged).unwrap(),
    )
    .unwrap();
    let fresh_state = tempfile::tempdir().unwrap();
    let fresh = open(&test_registry, fresh_state.path());
    let refreshed = fresh.refresh(now()).await;
    assert!(
        matches!(
            refreshed.blocklist,
            Some(fyx_registry::CacheUpdate::Keep(
                fyx_registry::KeepReason::Invalid(_)
            ))
        ),
        "{:?}",
        refreshed.blocklist
    );
}

/// Plan 3.5: the page's hub opens the registry from `fyx.extensions.registry_url` on first use
/// and lists the index with what the installed extensions may do.
#[gpui::test]
async fn test_hub_lists_the_index(cx: &mut gpui::TestAppContext) {
    use crate::fyx_extensions_page::RegistryHub;
    cx.executor().allow_parking();
    crate::tests::init_test(cx);
    let mut test_registry = TestRegistry::new();
    let package = crate::install_store::test_support::package("demo", "1.0.0", b"component").await;
    test_registry.publish("demo", "1.0.0", &package, &["ui:status-bar"]);
    // integration-5 R1: what the listing offers is what the install plan allows.
    let fresh = crate::install_store::test_support::package("fresh", "1.0.0", b"component").await;
    test_registry.publish_at("fresh", "1.0.0", &fresh, &[], now());
    let bad = crate::install_store::test_support::package("bad", "1.0.0", b"component").await;
    test_registry.publish("bad", "1.0.0", &bad, &[]);
    test_registry.write_blocklist(blocklist_blocking("bad", 1));
    let url = format!("file:///{}", test_registry.dir.path().display()).replace('\\', "/");
    cx.update(|cx| {
        gpui::BorrowAppContext::update_global::<settings::SettingsStore, _>(cx, |store, cx| {
            store.update_user_settings(cx, |settings| {
                settings
                    .fyx
                    .get_or_insert_default()
                    .extensions
                    .get_or_insert_default()
                    .registry_url = Some(url.clone())
            })
        });
        crate::fyx_extensions_page::init(cx);
        let fs: Arc<dyn fs::Fs> = fs::RealFs::new(None, cx.background_executor().clone());
        <dyn fs::Fs>::set_global(fs, cx);
    });
    let hub = cx.update(|cx| RegistryHub::global(cx).expect("hub"));
    let state = tempfile::tempdir().unwrap();
    hub.update(cx, |hub, _| {
        hub.set_state_dir(RegistryStateDir(state.path().to_path_buf()))
    });
    hub.read_with(cx, |hub, _| {
        assert!(hub.listing.is_empty(), "nothing before first use")
    });

    hub.update(cx, |hub, cx| hub.refresh(cx));
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while hub.read_with(cx, |hub, _| hub.is_refreshing()) && std::time::Instant::now() < deadline {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(10));
    }
    hub.read_with(cx, |hub, _| {
        assert_eq!(hub.off_reason(), None);
        assert_eq!(hub.unavailable, None);
        let mut listed: Vec<(&str, Option<&str>)> = hub
            .listing
            .iter()
            .map(|listed| {
                let refusal = match &listed.refusal {
                    None => None,
                    Some(fyx_registry::InstallRefusal::Blocked { .. }) => Some("blocked"),
                    Some(fyx_registry::InstallRefusal::TooNew { .. }) => Some("too new"),
                    Some(fyx_registry::InstallRefusal::NotInIndex) => Some("not in index"),
                };
                (listed.release.id.as_str(), refusal)
            })
            .collect();
        listed.sort();
        assert_eq!(
            listed,
            [
                ("bad", Some("blocked")),
                ("demo", None),
                ("fresh", Some("too new"))
            ]
        );
    });
    // The install path refuses the same releases, with no way around it.
    let registry = hub.update(cx, |hub, cx| hub.registry(cx).expect("registry"));
    for (id, installable) in [("demo", true), ("fresh", false), ("bad", false)] {
        let id = ExtensionId::new(id).unwrap();
        let result = registry.download_for_install(&id, now()).await;
        assert_eq!(result.is_ok(), installable, "{:?}", result.err());
    }
    // A refresh is a check: the 6 h timer counts from it, across restarts.
    let last = RegistryStateDir(state.path().to_path_buf()).last_check();
    assert!(last.is_some_and(|last| now().0 - last.0 < 60), "{last:?}");

    // Day 2: a new block of "demo". The automatic refresh (the page opening) waits until the
    // block list is due; the Refresh button fetches it now.
    test_registry.write_blocklist(blocklist_blocking("demo", 2));
    let blocked = |hub: &RegistryHub| {
        hub.listing.iter().any(|listed| {
            listed.release.id.as_str() == "demo"
                && matches!(
                    listed.refusal,
                    Some(fyx_registry::InstallRefusal::Blocked { .. })
                )
        })
    };
    let wait = |cx: &mut gpui::TestAppContext| {
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while hub.read_with(cx, |hub, _| hub.is_refreshing())
            && std::time::Instant::now() < deadline
        {
            cx.run_until_parked();
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    hub.update(cx, |hub, cx| hub.refresh(cx));
    wait(cx);
    assert!(!hub.read_with(cx, |hub, _| blocked(hub)), "not due yet");
    hub.update(cx, |hub, cx| hub.refresh_now(cx));
    wait(cx);
    assert!(
        hub.read_with(cx, |hub, _| blocked(hub)),
        "Refresh fetches it"
    );
}

/// Day 2: after a block of the installed version, the Available row (an older version, still
/// installable) says which version is installed and that it is disabled.
#[test]
fn test_available_row_of_a_blocked_install() {
    use crate::fyx_extensions_page::installed_row_state;
    let version = |text: &str| semver::Version::parse(text).unwrap();
    let (label, note) = installed_row_state(
        &version("0.1.0"),
        &version("0.1.1"),
        Some("blocked: crashes on start"),
    );
    assert_eq!(label, "Disabled");
    let (note, color) = note.expect("a note");
    assert_eq!(note, "Installed 0.1.1: disabled, blocked: crashes on start");
    assert_eq!(color, ui::Color::Error);
    assert_eq!(
        installed_row_state(&version("0.1.0"), &version("0.1.1"), None),
        ("Installed 0.1.1".to_string(), None)
    );
    assert_eq!(
        installed_row_state(&version("0.1.1"), &version("0.1.1"), None),
        ("Installed".to_string(), None)
    );
}

/// Plan 3.4: the update check updates a registry install, rolls a blocked version back,
/// disables one with nothing to roll back to, and enables it again when the block lifts.
#[gpui::test]
async fn test_update_check_applies_the_plan(cx: &mut gpui::TestAppContext) {
    use crate::fyx_extensions_page::apply_plan;
    cx.executor().allow_parking();
    let mut test_registry = TestRegistry::new();
    let v1 = crate::install_store::test_support::package("demo", "1.0.0", b"one").await;
    let v2 = crate::install_store::test_support::package("demo", "1.1.0", b"two").await;
    test_registry.publish("demo", "1.0.0", &v1, &["ui:status-bar"]);
    let store_dir = tempfile::tempdir().unwrap();
    let store = crate::install_store::InstallStore::new(store_dir.path().to_path_buf());
    let fs: Arc<dyn fs::Fs> = fs::RealFs::new(None, cx.executor());

    // A fresh client each round, so the block list is due every time.
    let check = |test_registry: &TestRegistry| {
        let state = tempfile::tempdir().unwrap();
        let registry = open(test_registry, state.path());
        let store = store.clone();
        let fs = fs.clone();
        async move {
            registry.refresh(now()).await;
            let installed = store.installed_extensions(fs.as_ref()).await.unwrap();
            let client = registry.client().lock().await;
            let actions = client.plan_updates(&installed, now());
            let updates: Vec<Release> = match client.index(now()) {
                Ok(index) => actions
                    .iter()
                    .filter_map(|action| match &action.action {
                        fyx_registry::Action::Update { to, .. } => {
                            index.release(&action.id, to).cloned()
                        }
                        _ => None,
                    })
                    .collect(),
                Err(_) => Vec::new(),
            };
            let blocks_checked = crate::fyx_extensions_page::lifts_blocks(&client, now());
            drop(client);
            let applied = apply_plan(
                &registry,
                &store,
                fs.as_ref(),
                &actions,
                &updates,
                blocks_checked,
            )
            .await;
            drop(state);
            applied
        }
    };
    let active = |store: &crate::install_store::InstallStore, fs: &Arc<dyn fs::Fs>| {
        let (store, fs) = (store.clone(), fs.clone());
        async move {
            let state = store.state(fs.as_ref()).await.unwrap();
            let entry = state.extensions["demo"].clone();
            (entry.active.to_string(), entry.disabled)
        }
    };

    // Installed from the registry at 1.0.0; 1.1.0 is published.
    let source = crate::install_store::VersionSource {
        origin: Origin::Registry,
        package_sha256: Some(Sha256Digest::of(&v1)),
    };
    store
        .install_package_bytes(v1.clone(), "demo", source, None, fs.as_ref())
        .await
        .unwrap();
    test_registry.publish("demo", "1.1.0", &v2, &["ui:status-bar"]);
    let applied = check(&test_registry).await.lines;
    assert_eq!(applied, ["updated demo to 1.1.0"]);
    assert_eq!(active(&store, &fs).await, ("1.1.0".into(), None));

    // 1.1.0 is blocked: back to 1.0.0.
    let mut block = blocklist_blocking("demo", 1);
    block.entries[0].versions = fyx_registry::VersionRange::parse("=1.1.0").unwrap();
    test_registry.write_blocklist(block);
    let outcome = check(&test_registry).await;
    // The window is told, with the block list's reason.
    assert_eq!(
        outcome.blocks,
        [crate::fyx_extensions_page::BlockNotice {
            extension_id: "demo".into(),
            what: "rolled back to 1.0.0".into(),
            reason: "Steals tokens".into(),
        }]
    );
    let applied = outcome.lines;
    assert_eq!(applied.len(), 1, "{applied:?}");
    assert!(
        applied[0].starts_with("rolled demo back to 1.0.0"),
        "{applied:?}"
    );
    assert_eq!(active(&store, &fs).await.0, "1.0.0");

    // Every version blocked: nothing to roll back to, so it is disabled and not loaded.
    test_registry.write_blocklist(blocklist_blocking("demo", 2));
    let outcome = check(&test_registry).await;
    assert_eq!(outcome.blocks[0].what, "disabled");
    let applied = outcome.lines;
    assert!(
        applied.iter().any(|line| line.starts_with("disabled demo")),
        "{applied:?}"
    );
    assert!(active(&store, &fs).await.1.is_some());
    assert!(
        store.active(fs.as_ref()).await.unwrap().is_empty(),
        "a disabled id does not load"
    );

    // The block lifts: 1.1.0 is installable again, so it updates, which enables it too.
    let mut lifted = blocklist_blocking("demo", 3);
    lifted.entries.clear();
    test_registry.write_blocklist(lifted);
    let applied = check(&test_registry).await.lines;
    assert_eq!(applied, ["updated demo to 1.1.0"]);
    assert_eq!(active(&store, &fs).await, ("1.1.0".into(), None));

    // A lifted block with no newer release enables the version in place.
    store
        .set_disabled("demo", Some("blocked: test".into()), fs.as_ref())
        .await
        .unwrap();
    let applied = check(&test_registry).await.lines;
    assert_eq!(applied, ["enabled demo again: no longer blocked"]);
    assert_eq!(active(&store, &fs).await.1, None);

    // integration-6 N4: with no verified list (here none can be fetched; the same as a cache
    // dropped after a key rotation), "no list" is not "nothing blocked": it stays disabled.
    store
        .set_disabled("demo", Some("blocked: test".into()), fs.as_ref())
        .await
        .unwrap();
    std::fs::remove_file(test_registry.dir.path().join("blocklist.json")).unwrap();
    let applied = check(&test_registry).await.lines;
    assert!(applied.is_empty(), "{applied:?}");
    assert!(active(&store, &fs).await.1.is_some(), "still disabled");
}

/// Plan 3.4: a block that changes an installed extension is shown in the active window, with
/// the extension's name and the block list's reason, as Fyx's own notice.
#[gpui::test]
async fn test_block_notice_goes_to_the_active_window(cx: &mut gpui::TestAppContext) {
    use gpui::AppContext as _;
    crate::tests::init_test(cx);
    let registry = cx.new(|_| crate::UiExtensionRegistry::trusting());
    let workspace = cx.new(|_| ()).entity_id();
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let _subscription = cx.update(|cx| {
        let events = events.clone();
        cx.subscribe(&registry, move |_, event: &crate::RegistryEvent, _| {
            events.borrow_mut().push(event.clone())
        })
    });
    let notice = crate::fyx_extensions_page::BlockNotice {
        extension_id: "demo".into(),
        what: "disabled".into(),
        reason: "Steals tokens".into(),
    };
    registry.update(cx, |registry, cx| {
        registry.notify_block("Demo", &notice, cx)
    });
    assert!(events.borrow().is_empty(), "no window: the log has it");
    registry.update(cx, |registry, cx| {
        registry.set_active_workspace(workspace);
        registry.notify_block("Demo", &notice, cx)
    });
    let events = events.borrow();
    let [
        crate::RegistryEvent::HostNotice {
            title,
            message,
            workspace: target,
            ..
        },
    ] = events.as_slice()
    else {
        panic!("{events:?}");
    };
    assert_eq!(title.as_ref(), "Demo was disabled");
    assert!(
        message.contains("Steals tokens") && message.contains("demo"),
        "{message}"
    );
    assert_eq!(*target, workspace);
}

/// What `script/fyx-test-registry` writes is what a debug build accepts: run with
/// `FYX_TEST_REGISTRY=<its OUT_DIR> cargo nextest run -p ui_ext_host --run-ignored all
/// test_script_registry`.
#[gpui::test]
#[ignore = "needs the output of script/fyx-test-registry"]
async fn test_script_registry(cx: &mut gpui::TestAppContext) {
    cx.executor().allow_parking();
    let dir = std::env::var("FYX_TEST_REGISTRY").expect("FYX_TEST_REGISTRY");
    let state = tempfile::tempdir().unwrap();
    let registry = Registry::open(
        compiled_keyset(),
        RegistryLocation::Directory(dir.into()),
        Arc::new(http_client::BlockedHttpClient),
        RegistryStateDir(state.path().to_path_buf()),
    )
    .expect("keys");
    let outcome = registry.refresh(now()).await;
    assert_eq!(outcome.index, IndexRefresh::Updated);
    assert!(
        matches!(
            outcome.blocklist,
            Some(fyx_registry::CacheUpdate::Replace(_))
        ),
        "{:?}",
        outcome.blocklist
    );
    let ids: Vec<ExtensionId> = registry
        .client()
        .lock()
        .await
        .index(now())
        .expect("index")
        .extensions()
        .cloned()
        .collect();
    assert!(!ids.is_empty());
    for id in ids {
        let (release, bytes) = registry
            .download_for_install(&id, now())
            .await
            .unwrap_or_else(|error| panic!("{id}: {error:#}"));
        assert_eq!(bytes.len() as u64, release.package.size);
    }
}

/// Spec §4, failure 1: an update with a new T2 capability is never approved, then the old version
/// is blocked. The extension is disabled, not upgraded to the held version, and the page offers
/// "Approve and Update" or "Uninstall" (both actions are planned for it).
#[gpui::test]
async fn test_spec_failure_1_held_t2_update_then_blocked(cx: &mut gpui::TestAppContext) {
    use crate::fyx_extensions_page::apply_plan;
    cx.executor().allow_parking();
    let mut test_registry = TestRegistry::new();
    let v1 = crate::install_store::test_support::package("demo", "1.0.0", b"one").await;
    let v2 = crate::install_store::test_support::package("demo", "2.0.0", b"two").await;
    test_registry.publish("demo", "1.0.0", &v1, &["ui:status-bar"]);
    let store_dir = tempfile::tempdir().unwrap();
    let store = crate::install_store::InstallStore::new(store_dir.path().to_path_buf());
    let fs: Arc<dyn fs::Fs> = fs::RealFs::new(None, cx.executor());
    let source = crate::install_store::VersionSource {
        origin: Origin::Registry,
        package_sha256: Some(Sha256Digest::of(&v1)),
    };
    let first = Release::clone(
        registry_release(&test_registry, "demo", "1.0.0", cx)
            .await
            .as_ref()
            .unwrap(),
    );
    store
        .install_package_bytes(
            v1,
            "demo",
            source,
            Some(Permissions::of(&first)),
            fs.as_ref(),
        )
        .await
        .unwrap();

    // 2.0.0 adds network:fetch (T2): held, never installed by itself.
    test_registry.publish("demo", "2.0.0", &v2, &["ui:status-bar", "network:fetch"]);
    // Then 1.0.0 is blocked.
    let mut block = blocklist_blocking("demo", 1);
    block.entries[0].versions = fyx_registry::VersionRange::parse("=1.0.0").unwrap();
    test_registry.write_blocklist(block);

    let state = tempfile::tempdir().unwrap();
    let registry = open(&test_registry, state.path());
    registry.refresh(now()).await;
    let installed = store.installed_extensions(fs.as_ref()).await.unwrap();
    let actions = registry
        .client()
        .lock()
        .await
        .plan_updates(&installed, now());
    assert!(
        actions
            .iter()
            .any(|action| matches!(action.action, fyx_registry::Action::Disable { .. })),
        "{actions:?}"
    );
    assert!(
        actions.iter().any(|action| matches!(
            &action.action,
            fyx_registry::Action::Hold { reasons, .. }
                if reasons.iter().any(|reason| matches!(
                    reason,
                    fyx_registry::HoldReason::NeedsApproval(additions)
                        if additions.capabilities.contains("network:fetch")
                ))
        )),
        "the held update stays on offer: {actions:?}"
    );
    assert!(
        !actions
            .iter()
            .any(|action| matches!(action.action, fyx_registry::Action::Update { .. })),
        "never upgraded to the held version: {actions:?}"
    );
    let outcome = apply_plan(&registry, &store, fs.as_ref(), &actions, &[], true).await;
    assert_eq!(outcome.blocks[0].what, "disabled");
    let entry = store.state(fs.as_ref()).await.unwrap().extensions["demo"].clone();
    assert_eq!(entry.active, Version::new(1, 0, 0), "not upgraded");
    assert!(entry.disabled.is_some(), "disabled");
}

async fn registry_release(
    test_registry: &TestRegistry,
    id: &str,
    version: &str,
    _cx: &mut gpui::TestAppContext,
) -> Option<Release> {
    let state = tempfile::tempdir().unwrap();
    let registry = open(test_registry, state.path());
    registry.refresh(now()).await;
    let client = registry.client().lock().await;
    let index = client.index(now()).ok()?;
    index
        .release(&ExtensionId::new(id).ok()?, &Version::parse(version).ok()?)
        .cloned()
}

/// Plan 4.5: a first-use request goes to the active window as a prompt naming the extension and
/// the hosts; the answer resolves it. With no window it resolves `Ask` and records nothing.
#[gpui::test]
async fn test_first_use_prompt_goes_to_the_active_window(cx: &mut gpui::TestAppContext) {
    use futures::FutureExt as _;
    use gpui::AppContext as _;
    use ui_ext_wasm::permissions::PermissionState;
    crate::tests::init_test(cx);
    let registry = cx.new(|_| crate::UiExtensionRegistry::trusting());
    let checker = registry.update(cx, |registry, cx| {
        registry.start_permission_prompts(cx);
        registry.consent_mut().declare(
            "demo",
            vec![
                "network:fetch".into(),
                "network:fetch@api.example.com".into(),
            ],
        );
        registry.capability_check()
    });
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let _subscription = cx.update(|cx| {
        let events = events.clone();
        cx.subscribe(&registry, move |_, event: &crate::RegistryEvent, _| {
            events.borrow_mut().push(event.clone())
        })
    });

    // No window: Ask, nothing asked.
    let request = checker.request("demo", "network:fetch");
    cx.run_until_parked();
    assert_eq!(request.now_or_never(), Some(PermissionState::Ask));
    assert!(events.borrow().is_empty());

    let workspace = cx.new(|_| ()).entity_id();
    registry.update(cx, |registry, _| registry.set_active_workspace(workspace));
    let request = checker.request("demo", "network:fetch");
    cx.run_until_parked();
    assert!(matches!(
        events.borrow().as_slice(),
        [crate::RegistryEvent::PermissionPrompt { capability, workspace: target, .. }]
            if capability.as_ref() == "network:fetch" && *target == workspace
    ));
    let (title, detail) = registry.read_with(cx, |registry, _| {
        registry.permission_prompt_text("demo", "network:fetch")
    });
    assert_eq!(title, "Allow demo to connect to the internet?");
    assert!(detail.starts_with("Hosts: api.example.com"), "{detail}");
    registry.update(cx, |registry, cx| {
        registry.answer_permission("demo", "network:fetch", Some(true), false, cx)
    });
    assert_eq!(request.now_or_never(), Some(PermissionState::Granted));
    assert_eq!(
        registry.read_with(cx, |registry, _| registry.t2_state("demo", "network:fetch")),
        PermissionState::Granted
    );
}

/// Decision 3: the timed check runs 6 h after the last check (kept across restarts), and never
/// at startup.
#[test]
fn test_check_timer_counts_from_the_last_check() {
    use crate::fyx_extensions_page::RegistryHub;
    let state = tempfile::tempdir().unwrap();
    let dir = RegistryStateDir(state.path().to_path_buf());
    let started = std::time::Instant::now();
    let hub = |dir: &RegistryStateDir| {
        let mut hub = RegistryHub::default();
        hub.set_state_dir(dir.clone());
        hub
    };

    // Never checked: due, but not before the startup grace.
    let mut fresh = hub(&dir);
    assert!(fresh.check_due());
    assert!(fresh.next_check_in(started) >= Duration::from_secs(14 * 60));

    // Checked 5 h ago in an earlier run: due in about 1 h.
    dir.set_last_check(Timestamp(now().0 - 5 * 60 * 60))
        .unwrap();
    let mut restarted = hub(&dir);
    assert!(!restarted.check_due());
    let wait = restarted.next_check_in(started);
    assert!(
        wait > Duration::from_secs(55 * 60) && wait <= Duration::from_secs(60 * 60),
        "{wait:?}"
    );

    // Checked 7 h ago: due, after the grace.
    dir.set_last_check(Timestamp(now().0 - 7 * 60 * 60))
        .unwrap();
    let mut overdue = hub(&dir);
    assert!(overdue.check_due());
    assert!(overdue.next_check_in(started) >= Duration::from_secs(14 * 60));
}

/// integration-5 R7: a registry package must be the release its index entry names, asking for
/// nothing beyond what the entry (and so the install dialog) declares.
#[gpui::test]
async fn test_registry_package_must_match_its_release(cx: &mut gpui::TestAppContext) {
    use crate::install_store::test_support::{manifest, zip};
    cx.executor().allow_parking();
    let mut test_registry = TestRegistry::new();
    let honest = zip(&[
        (
            "extension.toml",
            manifest("demo", "1.0.0", "\"ui:status-bar\"").as_bytes(),
        ),
        ("fyx_ui.wasm", b"component"),
    ])
    .await;
    test_registry.publish("demo", "1.0.0", &honest, &["ui:status-bar"]);
    let greedy = zip(&[
        (
            "extension.toml",
            manifest(
                "greedy",
                "1.0.0",
                "\"ui:status-bar\", \"workspace:diagnostics\"",
            )
            .as_bytes(),
        ),
        ("fyx_ui.wasm", b"component"),
    ])
    .await;
    test_registry.publish("greedy", "1.0.0", &greedy, &["ui:status-bar"]);
    let impostor = zip(&[
        (
            "extension.toml",
            manifest("demo", "1.0.0", "\"ui:status-bar\"").as_bytes(),
        ),
        ("fyx_ui.wasm", b"component"),
    ])
    .await;
    test_registry.publish("impostor", "1.0.0", &impostor, &["ui:status-bar"]);
    let state = tempfile::tempdir().unwrap();
    let registry = open(&test_registry, state.path());
    registry.refresh(now()).await;
    let store_dir = tempfile::tempdir().unwrap();
    let store = crate::install_store::InstallStore::new(store_dir.path().to_path_buf());
    let fs: Arc<dyn fs::Fs> = fs::RealFs::new(None, cx.executor());

    for (id, ok, why) in [
        ("demo", true, ""),
        ("greedy", false, "workspace:diagnostics"),
        ("impostor", false, "not the release"),
    ] {
        let (release, bytes) = registry
            .download_for_install(&ExtensionId::new(id).unwrap(), now())
            .await
            .expect("verified download");
        let result = store
            .install_registry_package(bytes, &release, None, fs.as_ref())
            .await;
        assert_eq!(result.is_ok(), ok, "{id}: {result:?}");
        if let Err(error) = result {
            assert!(format!("{error:#}").contains(why), "{id}: {error:#}");
        }
    }
}

/// integration-5 R4: revoking a T1 permission takes it out of the registry install's approval
/// too, so the next update asks for it again instead of granting it silently.
#[gpui::test]
async fn test_revoke_takes_back_the_approval(cx: &mut gpui::TestAppContext) {
    use crate::consent::{ConsentRecord, Grant};
    use crate::install_store::test_support::{manifest, zip};
    use gpui::AppContext as _;
    cx.executor().allow_parking();
    crate::tests::init_test(cx);
    // 0.3 has no T1 capabilities (W2c A3), so the revoked one is T2.
    let capabilities = ["ui:status-bar", "workspace:edit"];
    let mut test_registry = TestRegistry::new();
    let package = zip(&[
        (
            "extension.toml",
            // T1 capabilities are 0.3's (W2c A4).
            manifest("demo", "1.0.0", "\"ui:status-bar\", \"workspace:edit\"")
                .replace("api_version = \"0.1.0\"", "api_version = \"0.3.0\"")
                .as_bytes(),
        ),
        ("fyx_ui.wasm", b"component"),
    ])
    .await;
    test_registry.publish("demo", "1.0.0", &package, &capabilities);
    let state = tempfile::tempdir().unwrap();
    let registry_client = open(&test_registry, state.path());
    registry_client.refresh(now()).await;
    let (release, bytes) = registry_client
        .download_for_install(&ExtensionId::new("demo").unwrap(), now())
        .await
        .unwrap();
    let store_dir = tempfile::tempdir().unwrap();
    let store = crate::install_store::InstallStore::new(store_dir.path().to_path_buf());
    let fs: Arc<dyn fs::Fs> = fs::RealFs::new(None, cx.executor());
    store
        .install_registry_package(
            bytes,
            &release,
            Some(Permissions::of(&release)),
            fs.as_ref(),
        )
        .await
        .unwrap();
    let consent_dir = tempfile::tempdir().unwrap();
    let registry = cx.new(|_| {
        crate::UiExtensionRegistry::new(consent_dir.path().join("consent.json"))
            .with_store(store.clone())
    });
    registry.update(cx, |registry, cx| {
        registry.grant_consent(
            "demo",
            ConsentRecord {
                capabilities: capabilities.iter().map(|c| c.to_string()).collect(),
                grant: Grant::Component {
                    sha256: "abc".into(),
                },
            },
            fs.clone(),
            cx,
        );
        // A T2 capability is granted at first use; allowed, it can be revoked. The answer is
        // saved through the global file system.
        <dyn fs::Fs>::set_global(fs.clone(), cx);
        // As a load declares them.
        registry
            .consent_mut()
            .declare("demo", capabilities.iter().map(|c| c.to_string()).collect());
        registry.allow_t2("demo", "workspace:edit", cx);
        registry
            .revoke("demo", "workspace:edit", fs.clone(), cx)
            .expect("revoked");
    });
    // Waits on the write itself: reading the state file while it is replaced (Windows'
    // `ReplaceFileW`) failed under load, and the write was not ordered with others before.
    let written = registry.read_with(cx, |registry, _| registry.approval_writes_done());
    written.await;
    let installed = store.installed_extensions(fs.as_ref()).await.unwrap();
    assert!(
        !installed[0]
            .approved
            .capabilities
            .contains("workspace:edit")
    );
    assert!(installed[0].approved.capabilities.contains("ui:status-bar"));
}

/// Two revokes in a row both reach the approval: the second write waits for the first, so it
/// cannot write back the capability the first one took out.
#[gpui::test]
async fn test_revokes_in_a_row_both_reach_the_approval(cx: &mut gpui::TestAppContext) {
    use crate::install_store::test_support::{manifest, zip};
    use gpui::AppContext as _;
    cx.executor().allow_parking();
    crate::tests::init_test(cx);
    let revoked = ["workspace:edit", "agent:observe"];
    let capabilities = ["ui:status-bar", revoked[0], revoked[1]];
    let mut test_registry = TestRegistry::new();
    let package = zip(&[
        (
            "extension.toml",
            manifest(
                "demo",
                "1.0.0",
                "\"ui:status-bar\", \"workspace:edit\", \"agent:observe\"",
            )
            .replace("api_version = \"0.1.0\"", "api_version = \"0.3.0\"")
            .as_bytes(),
        ),
        ("fyx_ui.wasm", b"component"),
    ])
    .await;
    test_registry.publish("demo", "1.0.0", &package, &capabilities);
    let state = tempfile::tempdir().unwrap();
    let registry_client = open(&test_registry, state.path());
    registry_client.refresh(now()).await;
    let (release, bytes) = registry_client
        .download_for_install(&ExtensionId::new("demo").unwrap(), now())
        .await
        .unwrap();
    let store_dir = tempfile::tempdir().unwrap();
    let store = crate::install_store::InstallStore::new(store_dir.path().to_path_buf());
    let fs: Arc<dyn fs::Fs> = fs::RealFs::new(None, cx.executor());
    store
        .install_registry_package(
            bytes,
            &release,
            Some(Permissions::of(&release)),
            fs.as_ref(),
        )
        .await
        .unwrap();
    let consent_dir = tempfile::tempdir().unwrap();
    let registry = cx.new(|_| {
        crate::UiExtensionRegistry::new(consent_dir.path().join("consent.json"))
            .with_store(store.clone())
    });
    registry.update(cx, |registry, cx| {
        <dyn fs::Fs>::set_global(fs.clone(), cx);
        registry
            .consent_mut()
            .declare("demo", capabilities.iter().map(|c| c.to_string()).collect());
        for capability in revoked {
            registry.allow_t2("demo", capability, cx);
        }
        for capability in revoked {
            registry
                .revoke("demo", capability, fs.clone(), cx)
                .expect("revoked");
        }
    });
    let written = registry.read_with(cx, |registry, _| registry.approval_writes_done());
    written.await;
    let installed = store.installed_extensions(fs.as_ref()).await.unwrap();
    let approved = &installed[0].approved.capabilities;
    for capability in revoked {
        assert!(!approved.contains(capability), "{approved:?}");
    }
    assert!(approved.contains("ui:status-bar"), "{approved:?}");
}

/// integration-6 R9: a release keyset in which one key has two signing roles is refused like
/// one that does not parse.
#[test]
fn test_release_keyset_needs_role_separation() {
    use crate::registry_service::{TEST_KEYSET_JSON, parse_release_keyset};
    assert!(parse_release_keyset(TEST_KEYSET_JSON).is_some());
    let merged = TEST_KEYSET_JSON.replacen("\"release\"", "\"release\", \"blocklist\"", 1);
    assert_ne!(merged, TEST_KEYSET_JSON);
    assert!(parse_release_keyset(&merged).is_none());
    assert!(parse_release_keyset("not json").is_none());
}

/// Owner's Q1 (W2c 226eea0e96): an update that starts running a Zed extension asks with the
/// warning among what it asks for.
#[test]
fn test_a_new_zed_extension_is_asked_for_with_the_warning() {
    use crate::fyx_extensions_page::describe_additions;
    let additions = fyx_registry::Additions {
        zed_extension: true,
        ..Default::default()
    };
    assert!(!additions.is_empty());
    assert_eq!(
        describe_additions(&additions),
        [crate::install::ZED_EXTENSION_WARNING]
    );
    assert!(describe_additions(&fyx_registry::Additions::default()).is_empty());
}
