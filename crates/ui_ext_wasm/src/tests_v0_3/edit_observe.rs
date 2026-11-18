//! `workspace:edit` and `agent:observe` end to end (design.md §10, §11, plan 4.8), with fakes for
//! the host's editor and agent review sides. The pure rules are tested in `ui_ext::edit` and
//! `ui_ext::observe`, the queue in `observe.rs`.

use std::{
    collections::HashMap,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use futures::channel::mpsc;

use super::*;
use crate::{
    edit::{EditError, EditTarget, Position, TextRange, WorkspaceEdit, WorkspaceEdits},
    network::IoEvent,
    observe::{AgentEvent, AgentReviews, Resolution, ReviewInfo, ReviewSource},
    permissions::{CapabilityCheck, CapabilityDenied, PermissionState},
};

/// A broker with one state per capability, a fixed answer to every prompt, and a change stream.
struct Broker {
    states: Mutex<HashMap<String, PermissionState>>,
    answer: PermissionState,
    prompts: AtomicUsize,
    subscribers: Mutex<Vec<mpsc::UnboundedSender<(String, PermissionState)>>>,
}

impl Broker {
    fn new(state: PermissionState, answer: PermissionState) -> Arc<Self> {
        Arc::new(Self {
            states: Mutex::new(
                ["workspace:edit", "agent:observe"]
                    .into_iter()
                    .map(|capability| (capability.to_string(), state))
                    .collect(),
            ),
            answer,
            prompts: AtomicUsize::new(0),
            subscribers: Mutex::new(Vec::new()),
        })
    }

    fn granted() -> Arc<Self> {
        Self::new(PermissionState::Granted, PermissionState::Granted)
    }

    fn revoke(&self, capability: &str) {
        self.states
            .lock()
            .unwrap()
            .insert(capability.into(), PermissionState::Denied);
        for subscriber in self.subscribers.lock().unwrap().iter() {
            subscriber
                .unbounded_send((capability.into(), PermissionState::Denied))
                .ok();
        }
    }
}

impl CapabilityCheck for Broker {
    fn check(&self, _: &str, _: &str) -> Result<(), CapabilityDenied> {
        Ok(())
    }

    fn state(&self, _: &str, capability: &str) -> PermissionState {
        self.states
            .lock()
            .unwrap()
            .get(capability)
            .copied()
            .unwrap_or(PermissionState::Unavailable)
    }

    fn request(
        &self,
        _: &str,
        capability: &str,
    ) -> futures::future::BoxFuture<'static, PermissionState> {
        self.prompts.fetch_add(1, Ordering::SeqCst);
        self.states
            .lock()
            .unwrap()
            .insert(capability.into(), self.answer);
        futures::future::ready(self.answer).boxed()
    }

    fn changes(&self, _: &str) -> mpsc::UnboundedReceiver<(String, PermissionState)> {
        let (sender, receiver) = mpsc::unbounded();
        self.subscribers.lock().unwrap().push(sender);
        receiver
    }
}

/// The editor side: records each edit it is given and answers `result`, or never answers
/// while `hold` is set.
#[derive(Default)]
struct FakeEdits {
    applied: Mutex<Vec<(String, WorkspaceEdit)>>,
    result: Mutex<Option<EditError>>,
    hold: AtomicBool,
    calls: AtomicUsize,
}

impl WorkspaceEdits for FakeEdits {
    fn apply(
        &self,
        extension_id: &str,
        edit: WorkspaceEdit,
    ) -> futures::future::BoxFuture<'static, Result<(), EditError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.hold.load(Ordering::SeqCst) {
            return futures::future::pending().boxed();
        }
        self.applied
            .lock()
            .unwrap()
            .push((extension_id.to_string(), edit));
        let result = match self.result.lock().unwrap().clone() {
            Some(error) => Err(error),
            None => Ok(()),
        };
        futures::future::ready(result).boxed()
    }
}

/// The agent review side: a fixed snapshot, and a sender per subscription for the test to feed.
#[derive(Default)]
struct FakeReviews {
    snapshot: Vec<ReviewInfo>,
    feeds: Mutex<Vec<mpsc::UnboundedSender<AgentEvent>>>,
}

impl FakeReviews {
    fn feed(&self) -> mpsc::UnboundedSender<AgentEvent> {
        wait_until(|| !self.feeds.lock().unwrap().is_empty());
        self.feeds.lock().unwrap().last().unwrap().clone()
    }
}

impl AgentReviews for FakeReviews {
    fn subscribe(
        &self,
    ) -> (
        Vec<ReviewInfo>,
        futures::stream::BoxStream<'static, AgentEvent>,
    ) {
        let (sender, receiver) = mpsc::unbounded();
        self.feeds.lock().unwrap().push(sender);
        (self.snapshot.clone(), receiver.boxed())
    }
}

fn review(review: u64, hunks: usize) -> ReviewInfo {
    ReviewInfo {
        review,
        session: 7,
        turn: 9,
        file: Some(crate::process::FileRef {
            worktree: Some(1),
            path: "src/main.rs".into(),
        }),
        source: ReviewSource::Write,
        rule: None,
        resolution: Resolution::Pending(hunks as u32),
        submitted: false,
        hunks: (0..hunks)
            .map(|_| ui_ext::observe::HunkInfo {
                old_start: 1,
                old_count: 1,
                new_start: 1,
                new_count: 1,
                old_text: "a".into(),
                new_text: "b".into(),
                decision: ui_ext::observe::HunkDecision::Pending,
                comment: None,
            })
            .collect(),
        truncated: false,
    }
}

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[derive(Default)]
struct Setup {
    broker: Option<Arc<Broker>>,
    edits: Option<Arc<FakeEdits>>,
    reviews: Option<Arc<FakeReviews>>,
    undeclared: bool,
}

impl Setup {
    fn granted() -> Self {
        Self {
            broker: Some(Broker::granted()),
            edits: Some(Arc::default()),
            reviews: Some(Arc::default()),
            undeclared: false,
        }
    }

    async fn load(&self, cx: &mut TestAppContext) -> UiExtension {
        let mut capabilities = vec![
            CAPABILITY_PANEL.to_string(),
            ui_ext::v0_2::CAPABILITY_STATUS_BAR.to_string(),
            ui_ext::v0_2::CAPABILITY_MODAL.to_string(),
        ];
        if !self.undeclared {
            capabilities.push("workspace:edit".into());
            capabilities.push("agent:observe".into());
        }
        load_with(
            LoadOptions {
                allow_staged: Some(true),
                capabilities: Some(capabilities),
                capability_check: self
                    .broker
                    .clone()
                    .map(|broker| broker as Arc<dyn CapabilityCheck>),
                workspace_edits: self
                    .edits
                    .clone()
                    .map(|edits| edits as Arc<dyn WorkspaceEdits>),
                agent_reviews: self
                    .reviews
                    .clone()
                    .map(|reviews| reviews as Arc<dyn AgentReviews>),
                ..LoadOptions::default()
            },
            cx,
        )
        .await
        .expect("loads")
    }
}

async fn command(extension: &UiExtension, command: &str) -> String {
    let reply = extension
        .handle_event_v0_2(SURFACE, UiEvent::InputChanged(1, command.into()))
        .await
        .unwrap_or_else(|error| panic!("{command}: {error:#}"));
    match &reply.value {
        Update::Replace(nodes) => match &nodes[0].props {
            Props::Text(text) => text.content.clone(),
            other => panic!("not a report: {other:?}"),
        },
        other => panic!("not a report: {other:?}"),
    }
}

/// The next outcome, through `handle-io`: what the guest pushed in answer.
async fn deliver(extension: &UiExtension, events: &mut mpsc::Receiver<IoEvent>) -> String {
    let event = events.next().await.expect("an outcome");
    let reply = extension.handle_io(event).await.expect("handle-io");
    match reply.effects.first() {
        Some(HostEffect::Push {
            update: Update::Replace(nodes),
            ..
        }) => match &nodes[0].props {
            Props::Text(text) => text.content.clone(),
            other => panic!("not a report: {other:?}"),
        },
        other => panic!("no push: {other:?}"),
    }
}

#[gpui::test]
async fn test_an_edit_applies_after_its_call(cx: &mut TestAppContext) {
    let setup = Setup::granted();
    let extension = setup.load(cx).await;
    let mut events = extension.take_io_events().expect("once");
    let id = command(
        &extension,
        "edit label=Rename_all src/a.rs@0.0-0.3=fn src/a.rs@2.0-2.0=use_x; #5@1.0-1.4=",
    )
    .await;
    assert_eq!(id, "1");
    assert_eq!(deliver(&extension, &mut events).await, "edit-done 1 ok");
    let applied = setup
        .edits
        .as_ref()
        .unwrap()
        .applied
        .lock()
        .unwrap()
        .clone();
    assert_eq!(applied.len(), 1);
    let (extension_id, edit) = &applied[0];
    assert_eq!(extension_id, "probe-v0-3");
    assert_eq!(edit.label, "Rename all");
    assert_eq!(edit.files.len(), 2, "two targets");
    assert_eq!(
        edit.files[0].target,
        EditTarget::File(crate::process::FileRef {
            worktree: None,
            path: "src/a.rs".into(),
        })
    );
    assert_eq!(edit.files[0].edits.len(), 2);
    assert_eq!(edit.files[0].edits[1].new_text, "use x;");
    assert_eq!(
        edit.files[0].edits[0].range,
        TextRange {
            start: Position { line: 0, column: 0 },
            end: Position { line: 0, column: 3 },
        }
    );
    assert_eq!(edit.files[1].target, EditTarget::Document(5));
    assert_eq!(edit.files[1].edits[0].new_text, "", "a deletion");
}

#[gpui::test]
async fn test_the_import_checks(cx: &mut TestAppContext) {
    let undeclared = Setup {
        undeclared: true,
        ..Setup::granted()
    }
    .load(cx)
    .await;
    assert_eq!(
        command(&undeclared, "edit a.rs@0.0-0.0=x").await,
        "not-permitted"
    );
    assert_eq!(command(&undeclared, "observe start").await, "not-permitted");

    let denied = Setup {
        broker: Some(Broker::new(
            PermissionState::Denied,
            PermissionState::Denied,
        )),
        ..Setup::granted()
    }
    .load(cx)
    .await;
    assert_eq!(command(&denied, "edit a.rs@0.0-0.0=x").await, "denied");
    assert_eq!(command(&denied, "observe start").await, "denied");

    let no_host = Setup {
        edits: None,
        reviews: None,
        ..Setup::granted()
    }
    .load(cx)
    .await;
    assert_eq!(
        command(&no_host, "edit a.rs@0.0-0.0=x").await,
        "unavailable"
    );
    assert_eq!(command(&no_host, "observe start").await, "unavailable");

    let setup = Setup::granted();
    let extension = setup.load(cx).await;
    for bad in [
        "edit ../outside.rs@0.0-0.0=x",
        "edit /etc/passwd@0.0-0.0=x",
        "edit a.rs@2.0-1.0=x",
        "edit label= a.rs@0.0-0.0=x",
    ] {
        assert_eq!(command(&extension, bad).await, "invalid-argument", "{bad}");
    }
    let too_many: Vec<String> = (0..65).map(|n| format!("f{n}.rs@0.0-0.0=x")).collect();
    assert_eq!(
        command(&extension, &format!("edit {}", too_many.join(" "))).await,
        "invalid-argument"
    );
    // Four in flight at most, until their outcome is delivered.
    setup
        .edits
        .as_ref()
        .unwrap()
        .hold
        .store(true, Ordering::SeqCst);
    for id in 1..=4 {
        assert_eq!(
            command(&extension, "edit a.rs@0.0-0.0=x").await,
            id.to_string()
        );
    }
    assert_eq!(
        command(&extension, "edit a.rs@0.0-0.0=x").await,
        "limit-exceeded"
    );
}

#[gpui::test]
async fn test_protected_paths_and_overlaps_never_reach_the_editor(cx: &mut TestAppContext) {
    let setup = Setup::granted();
    let extension = setup.load(cx).await;
    let mut events = extension.take_io_events().expect("once");
    for (edit, outcome) in [
        (
            "edit src/ok.rs@0.0-0.0=x .git/hooks/pre-commit@0.0-0.0=x",
            "not-allowed",
        ),
        ("edit .VSCode/tasks.json@0.0-0.0=x", "not-allowed"),
        ("edit .fyx/review.toml@0.0-0.0=x", "not-allowed"),
        ("edit a.rs@0.0-0.5=x a.rs@0.4-0.6=y", "invalid"),
    ] {
        let id = command(&extension, edit).await;
        assert_eq!(
            deliver(&extension, &mut events).await,
            format!("edit-done {id} err {outcome}"),
            "{edit}"
        );
    }
    assert_eq!(
        setup.edits.as_ref().unwrap().calls.load(Ordering::SeqCst),
        0
    );
}

#[gpui::test]
async fn test_editor_errors_reach_the_guest(cx: &mut TestAppContext) {
    let setup = Setup::granted();
    *setup.edits.as_ref().unwrap().result.lock().unwrap() =
        Some(EditError::Conflict("a.rs changed".into()));
    let extension = setup.load(cx).await;
    let mut events = extension.take_io_events().expect("once");
    let id = command(&extension, "edit expect=3 a.rs@0.0-0.0=x").await;
    assert_eq!(
        deliver(&extension, &mut events).await,
        format!("edit-done {id} err conflict")
    );
    let applied = setup
        .edits
        .as_ref()
        .unwrap()
        .applied
        .lock()
        .unwrap()
        .clone();
    assert_eq!(applied[0].1.files[0].expected_version, Some(3));
}

#[gpui::test]
async fn test_a_trap_drops_the_edit(cx: &mut TestAppContext) {
    let setup = Setup::granted();
    let extension = setup.load(cx).await;
    extension
        .handle_event_v0_2(
            SURFACE,
            UiEvent::InputChanged(1, "edit-trap a.rs@0.0-0.0=x".into()),
        )
        .await
        .expect_err("traps");
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        setup.edits.as_ref().unwrap().calls.load(Ordering::SeqCst),
        0
    );
}

#[gpui::test]
async fn test_consent_is_asked_when_the_edit_applies(cx: &mut TestAppContext) {
    let setup = Setup {
        broker: Some(Broker::new(PermissionState::Ask, PermissionState::Denied)),
        ..Setup::granted()
    };
    let extension = setup.load(cx).await;
    let mut events = extension.take_io_events().expect("once");
    let id = command(&extension, "edit a.rs@0.0-0.0=x").await;
    assert_eq!(
        deliver(&extension, &mut events).await,
        format!("edit-done {id} err denied")
    );
    assert_eq!(
        setup
            .broker
            .as_ref()
            .unwrap()
            .prompts
            .load(Ordering::SeqCst),
        1
    );
    assert_eq!(
        setup.edits.as_ref().unwrap().calls.load(Ordering::SeqCst),
        0
    );
    assert_eq!(command(&extension, "edit a.rs@0.0-0.0=x").await, "denied");
}

#[gpui::test]
async fn test_a_revocation_ends_unapplied_edits_with_denied(cx: &mut TestAppContext) {
    let setup = Setup::granted();
    let edits = setup.edits.clone().unwrap();
    edits.hold.store(true, Ordering::SeqCst);
    let extension = setup.load(cx).await;
    let mut events = extension.take_io_events().expect("once");
    let id = command(&extension, "edit a.rs@0.0-0.0=x").await;
    wait_until(|| edits.calls.load(Ordering::SeqCst) == 1);
    setup.broker.as_ref().unwrap().revoke("workspace:edit");
    let mut outcomes = vec![
        deliver(&extension, &mut events).await,
        deliver(&extension, &mut events).await,
    ];
    outcomes.sort();
    assert_eq!(
        outcomes,
        [
            format!("edit-done {id} err denied"),
            "permission-changed T2Capability::WorkspaceEdit PermissionState::Denied".to_string(),
        ]
    );
}

#[gpui::test]
async fn test_observe_feeds_a_snapshot_then_changes(cx: &mut TestAppContext) {
    let reviews = Arc::new(FakeReviews {
        snapshot: vec![review(100, 2), review(200, 0)],
        ..FakeReviews::default()
    });
    let setup = Setup {
        reviews: Some(reviews.clone()),
        ..Setup::granted()
    };
    let extension = setup.load(cx).await;
    let mut events = extension.take_io_events().expect("once");
    assert_eq!(command(&extension, "observe start").await, "done");
    // Opaque ids, one map per extension: review 100 → 1, session 7 → 2, turn 9 → 3, review 200 → 4.
    assert_eq!(
        deliver(&extension, &mut events).await,
        "agent snapshot 1:2:3:2:false,4:2:3:0:false"
    );
    let feed = reviews.feed();
    assert_eq!(
        command(&extension, "observe start").await,
        "done",
        "again: no-op"
    );
    feed.unbounded_send(AgentEvent::Updated(review(100, 1)))
        .unwrap();
    assert_eq!(
        deliver(&extension, &mut events).await,
        "agent updated 1 2 3 1 false"
    );
    feed.unbounded_send(AgentEvent::Closed(200)).unwrap();
    assert_eq!(deliver(&extension, &mut events).await, "agent closed 4");
    feed.unbounded_send(AgentEvent::TurnEnded(9, Vec::new()))
        .unwrap();
    assert_eq!(
        deliver(&extension, &mut events).await,
        "agent turn-ended 3 0"
    );
    feed.unbounded_send(AgentEvent::ReviewMode(true)).unwrap();
    assert_eq!(
        deliver(&extension, &mut events).await,
        "agent review-mode true"
    );
    assert_eq!(reviews.feeds.lock().unwrap().len(), 1, "one subscription");

    assert_eq!(command(&extension, "observe stop").await, "done");
    wait_until(|| feed.is_closed());
}

#[gpui::test]
async fn test_a_revocation_or_a_trap_stops_observing(cx: &mut TestAppContext) {
    let reviews = Arc::new(FakeReviews::default());
    let setup = Setup {
        reviews: Some(reviews.clone()),
        ..Setup::granted()
    };
    let extension = setup.load(cx).await;
    let mut events = extension.take_io_events().expect("once");
    assert_eq!(command(&extension, "observe start").await, "done");
    assert_eq!(deliver(&extension, &mut events).await, "agent snapshot ");
    let feed = reviews.feed();
    setup.broker.as_ref().unwrap().revoke("agent:observe");
    assert_eq!(
        deliver(&extension, &mut events).await,
        "permission-changed T2Capability::AgentObserve PermissionState::Denied"
    );
    wait_until(|| feed.is_closed());

    let reviews = Arc::new(FakeReviews::default());
    let setup = Setup {
        reviews: Some(reviews.clone()),
        ..Setup::granted()
    };
    let extension = setup.load(cx).await;
    assert_eq!(command(&extension, "observe start").await, "done");
    let feed = reviews.feed();
    extension
        .handle_event_v0_2(SURFACE, UiEvent::InputChanged(1, "fetch-trap x".into()))
        .await
        .expect_err("traps");
    wait_until(|| feed.is_closed());

    // A reload (the last clone dropped) stops it too.
    let reviews = Arc::new(FakeReviews::default());
    let setup = Setup {
        reviews: Some(reviews.clone()),
        ..Setup::granted()
    };
    let extension = setup.load(cx).await;
    assert_eq!(command(&extension, "observe start").await, "done");
    let feed = reviews.feed();
    drop(extension);
    wait_until(|| feed.is_closed());
}

#[gpui::test]
async fn test_updates_coalesce_while_the_guest_lags(cx: &mut TestAppContext) {
    let reviews = Arc::new(FakeReviews {
        snapshot: vec![review(100, 0)],
        ..FakeReviews::default()
    });
    let setup = Setup {
        reviews: Some(reviews.clone()),
        ..Setup::granted()
    };
    let extension = setup.load(cx).await;
    let mut events = extension.take_io_events().expect("once");
    assert_eq!(command(&extension, "observe start").await, "done");
    let feed = reviews.feed();
    for hunks in 0..1000 {
        feed.unbounded_send(AgentEvent::Updated(review(100, hunks % 3)))
            .unwrap();
    }
    feed.unbounded_send(AgentEvent::Updated(review(100, 5)))
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    drop(feed);
    let mut delivered = Vec::new();
    while let Some(Some(event)) = events.next().now_or_never() {
        delivered.push(event);
    }
    // The channel holds its capacity plus one per sender; the 1001 updates fold into the rest.
    assert!(
        delivered.len() <= crate::network::MAX_QUEUED_IO_EVENTS + 4,
        "{} delivered",
        delivered.len()
    );
    match delivered.last() {
        Some(IoEvent::Agent(AgentEvent::Updated(info))) => {
            assert_eq!(info.hunks.len(), 5, "the newest update is kept")
        }
        other => panic!("{other:?}"),
    }
}

/// The world gives no way to decide a review: no `agent-observe` function takes an argument.
#[test]
fn test_no_agent_observe_function_takes_a_review() {
    let wit = include_str!("../../../ui_ext/wit/since_v0.3.0/fyx-ui.wit");
    let start = wit
        .find("interface agent-observe {")
        .expect("the interface");
    let body = &wit[start..];
    let body = &body[..body.find("\n}").expect("its end")];
    let functions: Vec<&str> = body
        .lines()
        .map(str::trim)
        .filter(|line| line.contains(": func("))
        .collect();
    assert_eq!(
        functions,
        ["start: func() -> result<_, host-error>;", "stop: func();"]
    );
}
