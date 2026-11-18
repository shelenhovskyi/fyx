//! The 0.3 `permissions` interface (design.md §7): `query` reports the broker's state for
//! declared capabilities, `request` needs and uses up the gesture and prompts after the call,
//! and the answer comes back as `permission-changed`.

use std::{
    collections::HashMap,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use futures::channel::mpsc;

use super::*;
use crate::{
    network::IoEvent,
    permissions::{CapabilityCheck, CapabilityDenied, PermissionState},
};

/// A broker with one state per capability; a prompt answers `answer` and announces it.
struct Broker {
    states: Mutex<HashMap<String, PermissionState>>,
    answer: PermissionState,
    prompts: Mutex<Vec<String>>,
    subscribers: Mutex<Vec<mpsc::UnboundedSender<(String, PermissionState)>>>,
    queries: AtomicUsize,
}

impl Broker {
    fn new(states: &[(&str, PermissionState)], answer: PermissionState) -> Arc<Self> {
        Arc::new(Self {
            states: Mutex::new(
                states
                    .iter()
                    .map(|(capability, state)| (capability.to_string(), *state))
                    .collect(),
            ),
            answer,
            prompts: Mutex::new(Vec::new()),
            subscribers: Mutex::new(Vec::new()),
            queries: AtomicUsize::new(0),
        })
    }

    fn prompts(&self) -> Vec<String> {
        self.prompts.lock().unwrap().clone()
    }
}

impl CapabilityCheck for Broker {
    fn check(&self, _: &str, _: &str) -> Result<(), CapabilityDenied> {
        Ok(())
    }

    fn state(&self, _: &str, capability: &str) -> PermissionState {
        self.queries.fetch_add(1, Ordering::SeqCst);
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
        self.prompts.lock().unwrap().push(capability.to_string());
        self.states
            .lock()
            .unwrap()
            .insert(capability.into(), self.answer);
        for subscriber in self.subscribers.lock().unwrap().iter() {
            subscriber
                .unbounded_send((capability.into(), self.answer))
                .ok();
        }
        futures::future::ready(self.answer).boxed()
    }

    fn changes(&self, _: &str) -> mpsc::UnboundedReceiver<(String, PermissionState)> {
        let (sender, receiver) = mpsc::unbounded();
        self.subscribers.lock().unwrap().push(sender);
        receiver
    }
}

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Declares `network:fetch` and `process:spawn`, not `workspace:edit` or `agent:observe`.
async fn load_declaring(broker: Option<Arc<Broker>>, cx: &mut TestAppContext) -> UiExtension {
    load_with(
        LoadOptions {
            allow_staged: Some(true),
            capabilities: Some(vec![
                CAPABILITY_PANEL.to_string(),
                ui_ext::v0_2::CAPABILITY_STATUS_BAR.to_string(),
                ui_ext::v0_2::CAPABILITY_MODAL.to_string(),
                "network:fetch".into(),
                "process:spawn".into(),
            ]),
            capability_check: broker.map(|broker| broker as Arc<dyn CapabilityCheck>),
            ..LoadOptions::default()
        },
        cx,
    )
    .await
    .expect("loads")
}

fn report(update: &Update) -> String {
    match update {
        Update::Replace(nodes) => match &nodes[0].props {
            Props::Text(text) => text.content.clone(),
            other => panic!("not a report: {other:?}"),
        },
        other => panic!("not a report: {other:?}"),
    }
}

async fn command(extension: &UiExtension, command: &str) -> String {
    let reply = extension
        .handle_event_v0_2(SURFACE, UiEvent::InputChanged(1, command.into()))
        .await
        .unwrap_or_else(|error| panic!("{command}: {error:#}"));
    report(&reply.value)
}

async fn click(extension: &UiExtension, case: u32) -> String {
    let reply = extension
        .handle_event_v0_2(SURFACE, UiEvent::Click(case))
        .await
        .unwrap_or_else(|error| panic!("click {case}: {error:#}"));
    report(&reply.value)
}

#[gpui::test]
async fn test_query_reports_declared_states(cx: &mut TestAppContext) {
    let broker = Broker::new(
        &[
            ("network:fetch", PermissionState::Granted),
            ("process:spawn", PermissionState::Ask),
            // Not declared: the broker's state is never shown.
            ("workspace:edit", PermissionState::Granted),
        ],
        PermissionState::Granted,
    );
    let extension = load_declaring(Some(broker.clone()), cx).await;
    for (capability, state) in [
        ("network-fetch", "granted"),
        ("process-spawn", "ask"),
        ("workspace-edit", "unavailable"),
        ("agent-observe", "unavailable"),
    ] {
        assert_eq!(
            command(&extension, &format!("perm query {capability}")).await,
            state,
            "{capability}"
        );
    }
    // One call sees one answer per capability.
    let before = broker.queries.load(Ordering::SeqCst);
    assert_eq!(
        click(&extension, 11).await,
        "granted | ask | unavailable | unavailable | done"
    );
    assert_eq!(
        broker.queries.load(Ordering::SeqCst) - before,
        2,
        "each declared capability once, though process-spawn is read again by request"
    );

    let no_broker = load_declaring(None, cx).await;
    assert_eq!(
        command(&no_broker, "perm query network-fetch").await,
        "unavailable"
    );
}

#[gpui::test]
async fn test_request_needs_the_gesture_and_prompts_after_the_call(cx: &mut TestAppContext) {
    let broker = Broker::new(
        &[("network:fetch", PermissionState::Ask)],
        PermissionState::Granted,
    );
    let extension = load_declaring(Some(broker.clone()), cx).await;
    let mut events = extension.take_io_events().expect("once");
    // `input-changed` is no gesture.
    assert_eq!(
        command(&extension, "perm request network-fetch").await,
        "not-permitted"
    );
    assert_eq!(
        command(&extension, "perm request workspace-edit").await,
        "not-permitted",
        "not declared"
    );
    // A click: the first request uses the gesture up.
    assert_eq!(click(&extension, 10).await, "done | not-permitted");
    wait_until(|| broker.prompts().len() == 1);
    assert_eq!(broker.prompts(), ["network:fetch"]);
    let event = events.next().await.expect("the answer");
    assert_eq!(
        event,
        IoEvent::PermissionChanged("network:fetch".into(), PermissionState::Granted)
    );
    let reply = extension.handle_io(event).await.expect("handle-io");
    match reply.effects.first() {
        Some(HostEffect::Push { update, .. }) => assert_eq!(
            report(update),
            "permission-changed T2Capability::NetworkFetch PermissionState::Granted"
        ),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        command(&extension, "perm query network-fetch").await,
        "granted"
    );

    // Granted already: a request shows no prompt.
    assert_eq!(click(&extension, 10).await, "done | not-permitted");
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(broker.prompts().len(), 1);
}

#[gpui::test]
async fn test_denied_stays_denied(cx: &mut TestAppContext) {
    let broker = Broker::new(
        &[("network:fetch", PermissionState::Denied)],
        PermissionState::Granted,
    );
    let extension = load_declaring(Some(broker.clone()), cx).await;
    assert_eq!(click(&extension, 10).await, "done | not-permitted");
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        broker.prompts().is_empty(),
        "only the extensions page changes denied"
    );
    assert_eq!(
        command(&extension, "perm query network-fetch").await,
        "denied"
    );
}
