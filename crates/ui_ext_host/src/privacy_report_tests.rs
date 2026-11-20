use fs::FakeFs;
use gpui::TestAppContext;
use ui_ext_wasm::journal::{Journal as _, now_ms};

use super::*;
use crate::journal::JournalStore;

const DAY_MS: u64 = 24 * 60 * 60 * 1000;

fn entry(at_ms: u64, kind: JournalKind) -> JournalEntry {
    JournalEntry { at_ms, kind }
}

fn fetched(host: &str, count: u32) -> JournalKind {
    JournalKind::Fetched {
        host: host.into(),
        count,
    }
}

/// Uses are summed per kind and thing, the most used first; older than the window and
/// bookkeeping entries are left out.
#[test]
fn test_extension_uses() {
    let now = 30 * DAY_MS;
    let page = JournalPage {
        entries: vec![
            entry(now - 8 * DAY_MS, fetched("old.example", 1)),
            entry(now - DAY_MS, fetched("api.example.com", 1)),
            entry(now - DAY_MS, fetched("cdn.example.com", 1)),
            entry(now, fetched("api.example.com", 4)),
            entry(
                now,
                JournalKind::FileEdited {
                    path: "src/a.rs".into(),
                    count: 2,
                },
            ),
            entry(
                now,
                JournalKind::ProgramStarted {
                    program: "git".into(),
                    args: vec!["status".into()],
                    cwd: None,
                    capability: "process:spawn".into(),
                },
            ),
            entry(
                now,
                JournalKind::ConsentChanged {
                    item_key: "network:fetch".into(),
                    allowed: true,
                },
            ),
        ],
        incomplete: false,
    };
    let lines: Vec<String> = extension_uses(&page, now - 7 * DAY_MS)
        .iter()
        .map(UseLine::text)
        .collect();
    assert_eq!(
        lines,
        [
            "Wrote src/a.rs (2 times)",
            "Fetched from api.example.com (5 times)",
            "Fetched from cdn.example.com",
            "Ran git",
        ]
    );
}

/// Secret uses by session, denied and missing ones marked.
#[test]
fn test_agent_reports() {
    let secret_use = |session: Option<&str>, secret: &str, outcome| SecretUse {
        unix_seconds: 100,
        session: session.map(String::from),
        secret: secret.into(),
        outcome,
    };
    let reports = agent_reports(
        &[
            secret_use(Some("s1"), "GITHUB_TOKEN", SecretUseOutcome::Allowed),
            secret_use(Some("s2"), "NPM_TOKEN", SecretUseOutcome::Denied),
            secret_use(Some("s1"), "GITHUB_TOKEN", SecretUseOutcome::Allowed),
            secret_use(None, "AWS_KEY", SecretUseOutcome::Missing),
        ],
        0,
    );
    let shown: Vec<(Actor, Vec<String>)> = reports
        .into_iter()
        .map(|report| {
            (
                report.actor,
                report.uses.iter().map(UseLine::text).collect(),
            )
        })
        .collect();
    let agent = |session: Option<&str>| Actor::Agent {
        session: session.map(String::from),
    };
    assert_eq!(
        shown,
        [
            (
                agent(Some("s1")),
                vec!["Used the secret GITHUB_TOKEN (2 times)".to_string()]
            ),
            (
                agent(Some("s2")),
                vec!["Used the secret NPM_TOKEN (denied)".to_string()]
            ),
            (
                agent(None),
                vec!["Used the secret AWS_KEY (missing)".to_string()]
            ),
        ]
    );
    assert!(agent_reports(&[secret_use(None, "X", SecretUseOutcome::Allowed)], 101).is_empty());
}

/// The report lists what the journal recorded for an extension and its granted capability;
/// Revoke takes the grant back and the report drops its button.
#[gpui::test]
async fn test_report_and_revoke(cx: &mut TestAppContext) {
    crate::tests::init_test(cx);
    let dir = tempfile::tempdir().unwrap();
    let store = JournalStore::at(dir.path().join("journal"));
    let fs = FakeFs::new(cx.executor());
    cx.update(|cx| <dyn Fs>::set_global(fs.clone(), cx));
    let registry = cx.new(|_| UiExtensionRegistry::new(dir.path().join("consent.json")));
    registry.update(cx, |registry, cx| {
        registry.set_journal_store(store.clone());
        registry
            .consent_mut()
            .declare("demo", vec!["network:fetch".into()]);
        registry.allow_t2("demo", "network:fetch", cx);
    });
    store.record("demo", fetched("api.example.com", 1));
    store.record("demo", fetched("api.example.com", 3));
    registry.update(cx, |registry, cx| {
        registry.request_journal("demo", true, cx)
    });
    cx.run_until_parked();

    let report = |cx: &mut TestAppContext| {
        registry.read_with(cx, |registry, _| build(registry, None, now_ms()))
    };
    let reports = report(cx);
    assert_eq!(reports.len(), 1);
    assert!(matches!(
        &reports[0].actor,
        Actor::Extension { id, .. } if &**id == "demo"
    ));
    assert_eq!(
        reports[0]
            .uses
            .iter()
            .map(UseLine::text)
            .collect::<Vec<_>>(),
        ["Fetched from api.example.com (4 times)"]
    );
    assert_eq!(reports[0].revocable, ["network:fetch"]);

    let check = registry.read_with(cx, |registry, _| registry.capability_check());
    registry
        .update(cx, |registry, cx| {
            registry.revoke("demo", "network:fetch", fs.clone(), cx)
        })
        .unwrap();
    assert!(check.check("demo", "network:fetch").is_err(), "at once");
    let reports = report(cx);
    assert!(reports[0].revocable.is_empty());
    assert_eq!(reports[0].uses.len(), 1, "what it did stays listed");
}
