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
            published_at: Timestamp(now().0 - 2 * 24 * 60 * 60),
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
        let ids: Vec<&str> = hub
            .listing
            .iter()
            .map(|release| release.id.as_str())
            .collect();
        assert_eq!(ids, ["demo"]);
    });
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
            drop(client);
            let applied = apply_plan(&registry, &store, fs.as_ref(), &actions, &updates).await;
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
    let outcome = apply_plan(&registry, &store, fs.as_ref(), &actions, &[]).await;
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
