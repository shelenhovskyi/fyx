//! The staged 0.3 world against `fixtures/probe_v0_3`: the `images` interface (plan 4.10), image
//! nodes and their checks, and the 0.3 interfaces this build links as traps.

use gpui::TestAppContext;
use ui_ext::{
    image::{ImageFit, ImageFormat, ImageProps, ImageSource},
    v0_2::{Icon, Props, UiEvent, Update},
};

use super::*;

const SURFACE: &str = "probe";

mod edit_observe;
mod permissions;

fn probe_v0_3_wasm() -> Vec<u8> {
    crate::test_support::fixture_wasm("fyx_ui_test_probe_v0_3")
}

async fn load_with(options: LoadOptions, cx: &mut TestAppContext) -> Result<UiExtension> {
    cx.executor().allow_parking();
    cx.update(gpui_tokio::init);
    cx.update(|cx| {
        UiExtension::load_with_options(
            "probe-v0-3".into(),
            probe_v0_3_wasm(),
            options,
            &cx.to_async(),
        )
    })
    .await
    .map(|(extension, _pushed)| extension)
}

async fn load(cx: &mut TestAppContext) -> UiExtension {
    load_with(
        LoadOptions {
            allow_staged: Some(true),
            ..LoadOptions::default()
        },
        cx,
    )
    .await
    .unwrap_or_else(|error| panic!("the 0.3 probe failed to load: {error:#}"))
}

/// Runs probe case `case`; returns its report and effects.
async fn case(extension: &UiExtension, case: u32) -> (Vec<String>, Vec<HostEffect>) {
    let reply = extension
        .handle_event_v0_2(SURFACE, UiEvent::Click(case))
        .await
        .unwrap_or_else(|error| panic!("case {case}: {error:#}"));
    let report = match &reply.value {
        Update::Replace(nodes) => match &nodes[0].props {
            Props::Text(text) => text.content.clone(),
            other => panic!("not a report: {other:?}"),
        },
        other => panic!("not a report: {other:?}"),
    };
    (
        report.split(" | ").map(String::from).collect(),
        reply.effects,
    )
}

#[gpui::test]
async fn test_v0_3_probe_loads_as_frozen_v0_3(cx: &mut TestAppContext) {
    let extension = load(cx).await;
    assert_eq!(extension.api_version(), &Version::new(0, 3, 0));
    assert_eq!(extension.ui_api_version(), ApiVersion::V0_3_0);
    assert_eq!(extension.surfaces_v0_2().len(), 3);

    // Frozen (integration-6): Stable and Preview load it too.
    let extension = load_with(
        LoadOptions {
            allow_staged: Some(false),
            ..LoadOptions::default()
        },
        cx,
    )
    .await
    .expect("Stable and Preview load 0.3");
    assert_eq!(extension.api_version(), &Version::new(0, 3, 0));
}

#[gpui::test]
async fn test_v0_3_add_image(cx: &mut TestAppContext) {
    let extension = load(cx).await;

    let (results, effects) = case(&extension, 1).await;
    assert_eq!(results, ["1", "2"]);
    assert!(matches!(
        &effects[..],
        [
            HostEffect::AddImage {
                handle: 1,
                format: ImageFormat::Png,
                ..
            },
            HostEffect::AddImage {
                handle: 2,
                format: ImageFormat::Svg,
                ..
            },
        ]
    ));

    // The bomb, 1 MiB + 1 byte, an entity-declaring SVG and junk: all refused, nothing recorded.
    let (results, effects) = case(&extension, 2).await;
    assert_eq!(
        results,
        [
            "limit-exceeded",
            "limit-exceeded",
            "invalid-argument",
            "invalid-argument"
        ]
    );
    assert!(effects.is_empty());
}

#[gpui::test]
async fn test_v0_3_at_most_32_generated_images(cx: &mut TestAppContext) {
    let extension = load(cx).await;
    let (results, effects) = case(&extension, 3).await;
    assert_eq!(results, ["ok*32,limit-exceeded*2", "33"]);
    assert_eq!(effects.len(), 34);
    assert_eq!(effects[32], HostEffect::RemoveImage { handle: 1 });
}

#[gpui::test]
async fn test_v0_3_images_need_the_capability(cx: &mut TestAppContext) {
    let extension = load_with(
        LoadOptions {
            capabilities: Some(vec![
                CAPABILITY_PANEL.into(),
                ui_ext::v0_2::CAPABILITY_STATUS_BAR.into(),
                ui_ext::v0_2::CAPABILITY_MODAL.into(),
            ]),
            allow_staged: Some(true),
            ..LoadOptions::default()
        },
        cx,
    )
    .await
    .expect("loads");
    let (results, effects) = case(&extension, 1).await;
    assert_eq!(results, ["not-permitted", "not-permitted"]);
    assert!(effects.is_empty());
}

fn logo() -> Props {
    Props::Image(ImageProps {
        source: ImageSource::Asset("assets/logo.svg".into()),
        alt: "Logo".into(),
        width: Some(32),
        height: None,
        fit: ImageFit::Contain,
        tint: None,
    })
}

/// Design.md §12, errors: an `image` push is checked when it is applied, which for a push is the
/// call. A handle removed since is not an error; one never returned is.
#[gpui::test]
async fn test_v0_3_image_pushes(cx: &mut TestAppContext) {
    let extension = load(cx).await;
    let (results, effects) = case(&extension, 4).await;
    assert_eq!(
        results,
        [
            "ok",
            "ok",
            "ok",
            "invalid-argument",
            "invalid-argument",
            "invalid-argument",
            "invalid-argument",
            "invalid-argument",
            "invalid-argument",
            "invalid-argument",
            "ok",
            "ok"
        ]
    );
    let pushes: Vec<&Update> = effects
        .iter()
        .filter_map(|effect| match effect {
            HostEffect::Push { surface_id, update } if surface_id == SURFACE => Some(update),
            _ => None,
        })
        .collect();
    assert_eq!(pushes.len(), 5);
    assert_eq!(
        pushes[0],
        &Update::Replace(vec![ui_ext::v0_2::Node {
            id: 2,
            parent: None,
            props: logo(),
        }])
    );
    assert!(matches!(
        &effects[effects.len() - 3..],
        [
            HostEffect::Push { .. },
            HostEffect::RemoveImage { handle: 1 },
            HostEffect::Push { .. }
        ]
    ));
}

/// Replies are checked like pushes; a bad one is rejected (a resync), not fatal.
#[gpui::test]
async fn test_v0_3_image_replies(cx: &mut TestAppContext) {
    let extension = load(cx).await;
    let reply = extension
        .handle_event_v0_2(SURFACE, UiEvent::Click(5))
        .await
        .expect("an image in a panel");
    assert!(matches!(&reply.value, Update::Replace(nodes) if nodes[0].props == logo()));
    let rows = extension
        .list_rows_v0_2(SURFACE, 7, 0, 1)
        .await
        .expect("an image in a panel row");
    assert_eq!(rows.value[0].nodes[0].props, logo());

    for surface in ["status", "modal"] {
        let error = extension
            .handle_event_v0_2(surface, UiEvent::Click(5))
            .await
            .unwrap_err();
        assert!(
            format!("{error:#}").contains("can only be in a panel"),
            "{error:#}"
        );
        assert!(error.downcast_ref::<EffectReplyError>().is_some());
        let error = extension.mount_v0_2(surface).await.unwrap_err();
        assert!(
            error.downcast_ref::<EffectReplyError>().is_some(),
            "{error:#}"
        );
        let error = extension
            .list_rows_v0_2(surface, 7, 0, 1)
            .await
            .unwrap_err();
        assert!(
            error.downcast_ref::<EffectReplyError>().is_some(),
            "{error:#}"
        );
        let error = extension.mount_tree_v0_2(surface).await.unwrap_err();
        assert!(
            error.downcast_ref::<EffectReplyError>().is_some(),
            "{error:#}"
        );
        let error = extension
            .list_row_trees_v0_2(surface, 7, 0, 1)
            .await
            .unwrap_err();
        assert!(
            error.downcast_ref::<EffectReplyError>().is_some(),
            "{error:#}"
        );
    }
    // The panel mounts, and nothing above was fatal.
    extension.mount_v0_2(SURFACE).await.expect("mounts");
    assert_eq!(case(&extension, 1).await.0, ["1", "2"]);
}

/// Design.md §13: canvas pushes are checked when applied, like images: the props rules and
/// panels only. The 16-per-surface and no-list-row rules belong to the tree (`ui_ext`).
#[gpui::test]
async fn test_v0_3_canvas_pushes(cx: &mut TestAppContext) {
    let extension = load(cx).await;
    let (results, effects) = case(&extension, 8).await;
    assert_eq!(
        results,
        [
            "ok",
            "invalid-argument",
            "invalid-argument",
            "limit-exceeded",
            "invalid-argument",
            "invalid-argument"
        ]
    );
    let pushes: Vec<_> = effects
        .iter()
        .filter_map(|effect| match effect {
            HostEffect::Push {
                update: Update::Replace(nodes),
                ..
            } => Some(&nodes[0].props),
            _ => None,
        })
        .collect();
    let [Props::Canvas(canvas)] = &pushes[..] else {
        panic!("one canvas push: {pushes:?}");
    };
    assert_eq!(canvas.ops.len(), 6);
    assert_eq!(canvas.fit, ui_ext::canvas::CanvasFit::ScaleDown);
    assert_eq!(ui_ext::canvas::check_canvas_props(canvas), Ok(()));
}

/// A canvas reply is checked too (panels only), and `canvas-click` reaches the guest in canvas
/// coordinates.
#[gpui::test]
async fn test_v0_3_canvas_replies_and_clicks(cx: &mut TestAppContext) {
    let extension = load(cx).await;
    let reply = extension
        .handle_event_v0_2(SURFACE, UiEvent::Click(9))
        .await
        .expect("a canvas in a panel");
    assert!(
        matches!(&reply.value, Update::Replace(nodes) if matches!(nodes[0].props, Props::Canvas(_)))
    );
    let error = extension
        .handle_event_v0_2("modal", UiEvent::Click(9))
        .await
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("can only be in a panel"),
        "{error:#}"
    );

    let reply = extension
        .handle_event_v0_2(
            SURFACE,
            UiEvent::CanvasClick(ui_ext::canvas::CanvasClick {
                canvas: 1,
                x: 12.5.into(),
                y: 3.0.into(),
            }),
        )
        .await
        .expect("handled");
    assert!(
        matches!(&reply.value, Update::Replace(nodes)
        if nodes[0].props == Props::Text(ui_ext::v0_2::TextProps {
            content: "canvas-click 1 12.5 3".into(),
            ..Default::default()
        })),
        "{:?}",
        reply.value
    );
}

/// Plan 4.13's lift bench: a guest answers or pushes a canvas of `n` mixed ops, and the host
/// lifts, converts and checks it. Target: 1 ms per 10 000 ops in a release build. The round trip
/// also holds the guest's clone and lowering and one event's dispatch, so it is an upper bound;
/// `canvas 6` is the floor. Measured 2026-11-19 (W2c.md): 1.1–1.3 ms per 10 000 round trip, of
/// which the host's convert and checks are 0.16 ms and the rest is the canonical-ABI lift and the
/// guest's lowering; the 16384 cap stays. The assertion guards against a regression past 2 ms
/// per 10 000. Run it with
/// `cargo test -p ui_ext_wasm --release --lib canvas_lift_bench -- --ignored --nocapture`.
#[gpui::test]
#[ignore = "a bench: run it in release"]
async fn test_canvas_lift_bench(cx: &mut TestAppContext) {
    use std::time::{Duration, Instant};

    const RUNS: usize = 30;
    fn median(mut times: Vec<Duration>) -> Duration {
        times.sort();
        times[times.len() / 2]
    }

    let extension = load(cx).await;
    let mut lines = Vec::new();
    let mut misses = Vec::new();
    for count in [6usize, 1_000, 10_000, 16_384] {
        let command = format!("canvas {count}");
        let push_command = format!("canvas-push {count}");
        let (mut replies, mut pushes, mut checks) = (Vec::new(), Vec::new(), Vec::new());
        for run in 0..RUNS + 3 {
            let start = Instant::now();
            let reply = extension
                .handle_event_v0_2(SURFACE, UiEvent::InputChanged(1, command.clone()))
                .await
                .expect("a canvas reply");
            let reply_time = start.elapsed();
            let Update::Replace(nodes) = &reply.value else {
                panic!("not a canvas: {:?}", reply.value);
            };
            let Props::Canvas(canvas) = &nodes[0].props else {
                panic!("not a canvas: {:?}", nodes[0].props);
            };
            assert_eq!(canvas.ops.len(), count);
            let start = Instant::now();
            assert_eq!(ui_ext::canvas::check_canvas_props(canvas), Ok(()));
            let check_time = start.elapsed();

            let start = Instant::now();
            let pushed = extension
                .handle_event_v0_2(SURFACE, UiEvent::InputChanged(1, push_command.clone()))
                .await
                .expect("a canvas push");
            let push_time = start.elapsed();
            assert!(
                matches!(&pushed.value, Update::Replace(nodes)
                if nodes[0].props == Props::Text(ui_ext::v0_2::TextProps {
                    content: "ok".into(),
                    ..Default::default()
                })),
                "{:?}",
                pushed.value
            );
            // The first runs warm the guest's cache and the allocator.
            if run >= 3 {
                replies.push(reply_time);
                pushes.push(push_time);
                checks.push(check_time);
            }
        }
        let (reply, push, check) = (median(replies), median(pushes), median(checks));
        let per_10k = |time: Duration| time.as_secs_f64() * 1e3 * 10_000.0 / count as f64;
        lines.push(format!(
            "{count:>6} ops: reply {reply:>10.3?} ({:.3} ms/10k), push {push:>10.3?} ({:.3} ms/10k), check alone {check:>10.3?}",
            per_10k(reply),
            per_10k(push),
        ));
        if count >= 10_000 && per_10k(reply.max(push)) > 2.0 {
            misses.push(count);
        }
    }
    println!(
        "canvas lift bench, median of {RUNS}:
{}",
        lines.join(
            "
"
        )
    );
    if !cfg!(debug_assertions) {
        assert!(misses.is_empty(), "over 2 ms per 10 000 ops at {misses:?}");
    }
}

/// The smaller 0.3 (integration-5 A3) links every interface of its world: a 0.3 component keeps
/// 0.2's `place-decoration`. WASI imports at a newer patch version than the host's (std's
/// `HashMap` seed in the probe's `surfaces`) still resolve.
#[gpui::test]
async fn test_v0_3_keeps_place_decoration(cx: &mut TestAppContext) {
    let extension = load(cx).await;
    let (results, effects) = case(&extension, 6).await;
    assert_eq!(results, ["done"]);
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, HostEffect::PlaceDecoration { id, .. } if id == "probe-decoration")),
        "{effects:?}"
    );
}

#[test]
fn test_v0_3_frozen_icons_match_the_wit() {
    let mapped: Vec<Icon> = fyx_ui_v0_3::WIT_ICONS
        .iter()
        .map(|&icon| icon.into())
        .collect();
    assert_eq!(mapped, Icon::ALL);
}

/// Done criterion of plan 4.10: a 0.2 component cannot send `image`, because its world has none.
#[test]
fn test_v0_2_world_has_no_images() {
    let wit_v0_2 = include_str!("../../ui_ext/wit/since_v0.2.0/fyx-ui.wit");
    assert!(!wit_v0_2.contains("image-props") && !wit_v0_2.contains("interface images"));
    let wit_v0_3 = include_str!("../../ui_ext/wit/since_v0.3.0/fyx-ui.wit");
    assert!(wit_v0_3.contains("image(image-props)") && wit_v0_3.contains("interface images"));
}

/// `network:fetch` end to end (design.md §8, plan 4.6): the import's checks, the broker, consent
/// when the request is applied, delivery through `handle-io`, cancel, the caps, a trap and a
/// reload. The engine's own rules (caps, redirects, addresses) are tested in `network.rs`.
mod network_fetch {
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use super::*;
    use crate::{
        network::{IoEvent, test_server},
        permissions::{CapabilityCheck, CapabilityDenied, PermissionState},
    };

    /// A test permission broker: one state for `network:fetch`, a fixed answer to the prompt,
    /// and a change stream.
    struct Broker {
        state: Mutex<PermissionState>,
        answer: PermissionState,
        prompts: AtomicUsize,
        subscribers: Mutex<Vec<futures::channel::mpsc::UnboundedSender<(String, PermissionState)>>>,
    }

    impl Broker {
        fn new(state: PermissionState, answer: PermissionState) -> Arc<Self> {
            Arc::new(Self {
                state: Mutex::new(state),
                answer,
                prompts: AtomicUsize::new(0),
                subscribers: Mutex::new(Vec::new()),
            })
        }

        fn set(&self, state: PermissionState) {
            *self.state.lock().unwrap() = state;
        }

        /// The extensions page revokes the grant.
        fn revoke(&self) {
            self.set(PermissionState::Denied);
            for subscriber in self.subscribers.lock().unwrap().iter() {
                subscriber
                    .unbounded_send(("network:fetch".into(), PermissionState::Denied))
                    .ok();
            }
        }
    }

    impl CapabilityCheck for Broker {
        fn check(&self, extension_id: &str, capability: &str) -> Result<(), CapabilityDenied> {
            match self.state(extension_id, capability) {
                PermissionState::Granted => Ok(()),
                state => Err(CapabilityDenied {
                    extension_id: extension_id.into(),
                    capability: capability.into(),
                    reason: format!("{state:?}"),
                }),
            }
        }

        fn state(&self, _: &str, capability: &str) -> PermissionState {
            if capability == "network:fetch" {
                *self.state.lock().unwrap()
            } else {
                PermissionState::Unavailable
            }
        }

        fn request(
            &self,
            _: &str,
            capability: &str,
        ) -> futures::future::BoxFuture<'static, PermissionState> {
            assert_eq!(capability, "network:fetch");
            self.prompts.fetch_add(1, Ordering::SeqCst);
            self.set(self.answer);
            futures::future::ready(self.answer).boxed()
        }

        fn changes(
            &self,
            _: &str,
        ) -> futures::channel::mpsc::UnboundedReceiver<(String, PermissionState)> {
            let (sender, receiver) = futures::channel::mpsc::unbounded();
            self.subscribers.lock().unwrap().push(sender);
            receiver
        }
    }

    fn server(cx: &mut TestAppContext) -> test_server::Server {
        cx.executor().allow_parking();
        test_server::start_in_thread(|path, _| match path {
            "/hello" => Some(test_server::ok(b"hello")),
            _ => None,
        })
    }

    async fn load_network(
        port: u16,
        broker: Option<Arc<Broker>>,
        declare: bool,
        cx: &mut TestAppContext,
    ) -> UiExtension {
        let mut capabilities = vec![
            CAPABILITY_PANEL.to_string(),
            ui_ext::v0_2::CAPABILITY_STATUS_BAR.to_string(),
            ui_ext::v0_2::CAPABILITY_MODAL.to_string(),
        ];
        if declare {
            capabilities.push("network:fetch".into());
        }
        load_with(
            LoadOptions {
                allow_staged: Some(true),
                capabilities: Some(capabilities),
                network_domains: Some(vec![format!("localhost:{port}")]),
                capability_check: broker.map(|broker| broker as Arc<dyn CapabilityCheck>),
                ..LoadOptions::default()
            },
            cx,
        )
        .await
        .expect("loads")
    }

    async fn command(extension: &UiExtension, command: &str) -> Vec<String> {
        let reply = extension
            .handle_event_v0_2(SURFACE, UiEvent::InputChanged(1, command.into()))
            .await
            .unwrap_or_else(|error| panic!("{command}: {error:#}"));
        match &reply.value {
            Update::Replace(nodes) => match &nodes[0].props {
                Props::Text(text) => text.content.split(" | ").map(String::from).collect(),
                other => panic!("not a report: {other:?}"),
            },
            other => panic!("not a report: {other:?}"),
        }
    }

    /// The next outcome, through `handle-io`: what the guest pushed in answer.
    async fn deliver(
        extension: &UiExtension,
        events: &mut futures::channel::mpsc::Receiver<IoEvent>,
    ) -> String {
        let event = events.next().await.expect("an outcome");
        let reply = extension.handle_io(event).await.expect("handle-io");
        match &reply.effects[..] {
            [
                HostEffect::Push {
                    update: Update::Replace(nodes),
                    ..
                },
            ] => match &nodes[0].props {
                Props::Text(text) => text.content.clone(),
                other => panic!("not a report: {other:?}"),
            },
            other => panic!("not one push: {other:?}"),
        }
    }

    #[gpui::test]
    async fn test_fetch_and_handle_io(cx: &mut TestAppContext) {
        let server = server(cx);
        let broker = Broker::new(PermissionState::Granted, PermissionState::Granted);
        let extension = load_network(server.port, Some(broker.clone()), true, cx).await;
        let mut events = extension.take_io_events().expect("once");
        assert!(extension.take_io_events().is_none());
        let url = format!("http://localhost:{}/hello", server.port);
        assert_eq!(command(&extension, &format!("fetch {url}")).await, ["1"]);
        assert_eq!(
            deliver(&extension, &mut events).await,
            "fetch-done 1 200 hello"
        );
        // Ids never repeat.
        assert_eq!(command(&extension, &format!("fetch {url}")).await, ["2"]);
        assert_eq!(
            deliver(&extension, &mut events).await,
            "fetch-done 2 200 hello"
        );
        // The Extension Inspector's lines.
        let lines: Vec<_> = extension
            .diagnostics()
            .io
            .iter()
            .map(|record| {
                (
                    record.capability,
                    record.id,
                    record.what.clone(),
                    record.outcome.clone(),
                )
            })
            .collect();
        let what = format!("Get http://localhost:{}/hello", server.port);
        assert_eq!(
            lines,
            [
                (
                    "network:fetch",
                    1,
                    what.clone(),
                    Some("200 (5 bytes)".to_string())
                ),
                ("network:fetch", 2, what, Some("200 (5 bytes)".to_string())),
            ]
        );
    }

    /// Plan 7.6: fetches reach the usage journal for the privacy report, the first one per host
    /// at once and later ones summed when the instance ends.
    #[gpui::test]
    async fn test_fetches_reach_the_journal(cx: &mut TestAppContext) {
        #[derive(Default)]
        struct Recorder(Mutex<Vec<crate::journal::JournalKind>>);

        impl crate::journal::Journal for Recorder {
            fn record(&self, _: &str, kind: crate::journal::JournalKind) {
                self.0.lock().unwrap().push(kind);
            }
        }

        let server = server(cx);
        let broker = Broker::new(PermissionState::Granted, PermissionState::Granted);
        let journal = Arc::new(Recorder::default());
        let extension = load_with(
            LoadOptions {
                allow_staged: Some(true),
                capabilities: Some(vec![
                    CAPABILITY_PANEL.to_string(),
                    ui_ext::v0_2::CAPABILITY_STATUS_BAR.to_string(),
                    ui_ext::v0_2::CAPABILITY_MODAL.to_string(),
                    "network:fetch".into(),
                ]),
                network_domains: Some(vec![format!("localhost:{}", server.port)]),
                capability_check: Some(broker as Arc<dyn CapabilityCheck>),
                journal: Some(journal.clone()),
                ..LoadOptions::default()
            },
            cx,
        )
        .await
        .expect("loads");
        let mut events = extension.take_io_events().expect("once");
        let url = format!("http://localhost:{}/hello", server.port);
        for _ in 0..3 {
            command(&extension, &format!("fetch {url}")).await;
            deliver(&extension, &mut events).await;
        }
        let fetched = |count| crate::journal::JournalKind::Fetched {
            host: "localhost".into(),
            count,
        };
        assert_eq!(*journal.0.lock().unwrap(), [fetched(1)]);
        drop(events);
        drop(extension);
        // The instance ends on its own thread.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while journal.0.lock().unwrap().len() < 2 && std::time::Instant::now() < deadline {
            cx.run_until_parked();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(*journal.0.lock().unwrap(), [fetched(1), fetched(2)]);
    }

    /// Plan 4.6, done: an undeclared domain is `denied` at the import.
    #[gpui::test]
    async fn test_what_the_import_refuses(cx: &mut TestAppContext) {
        let server = server(cx);
        let broker = Broker::new(PermissionState::Granted, PermissionState::Granted);
        let extension = load_network(server.port, Some(broker.clone()), true, cx).await;
        assert_eq!(
            command(
                &extension,
                &format!(
                    "fetch https://undeclared.example/ http://localhost:{}/ https://user:pw@localhost:{}/",
                    server.port + 1,
                    server.port
                )
            )
            .await,
            ["denied", "denied", "invalid-argument"]
        );
        // The broker at every call.
        broker.set(PermissionState::Denied);
        assert_eq!(
            command(
                &extension,
                &format!("fetch http://localhost:{}/hello", server.port)
            )
            .await,
            ["denied"]
        );
        // Not declared in the manifest.
        let undeclared = load_network(server.port, Some(broker), false, cx).await;
        assert_eq!(
            command(
                &undeclared,
                &format!("fetch http://localhost:{}/hello", server.port)
            )
            .await,
            ["not-permitted"]
        );
    }

    /// T2 consent when the request is applied: `ask` shows the prompt and waits; "Don't allow"
    /// and no broker are `denied`; a revocation before delivery turns the outcome into `denied`.
    #[gpui::test]
    async fn test_consent(cx: &mut TestAppContext) {
        let server = server(cx);
        let url = format!("http://localhost:{}/hello", server.port);

        let refused = Broker::new(PermissionState::Ask, PermissionState::Denied);
        let extension = load_network(server.port, Some(refused.clone()), true, cx).await;
        let mut events = extension.take_io_events().expect("once");
        assert_eq!(command(&extension, &format!("fetch {url}")).await, ["1"]);
        assert_eq!(
            deliver(&extension, &mut events).await,
            "fetch-done 1 err denied"
        );
        assert_eq!(refused.prompts.load(Ordering::SeqCst), 1);
        assert_eq!(server.connections(), 0, "nothing was sent");

        let allowed = Broker::new(PermissionState::Ask, PermissionState::Granted);
        let extension = load_network(server.port, Some(allowed.clone()), true, cx).await;
        let mut events = extension.take_io_events().expect("once");
        assert_eq!(command(&extension, &format!("fetch {url}")).await, ["1"]);
        assert_eq!(
            deliver(&extension, &mut events).await,
            "fetch-done 1 200 hello"
        );

        // Revoked while the request ran: the guest gets `denied`, not the response.
        assert_eq!(command(&extension, &format!("fetch {url}")).await, ["2"]);
        let event = events.next().await.expect("an outcome");
        assert!(matches!(&event, IoEvent::FetchDone(2, Ok(_))));
        allowed.set(PermissionState::Denied);
        let reply = extension.handle_io(event).await.expect("handle-io");
        assert!(
            format!("{:?}", reply.effects).contains("fetch-done 2 err denied"),
            "{:?}",
            reply.effects
        );
    }

    /// A broker that predates T2 (only `check`, the default `state`) leaves T2 denied.
    #[gpui::test]
    async fn test_t2_is_denied_by_a_broker_without_t2(cx: &mut TestAppContext) {
        let server = server(cx);
        struct AllowCalls;
        impl CapabilityCheck for AllowCalls {
            fn check(&self, _: &str, _: &str) -> Result<(), CapabilityDenied> {
                Ok(())
            }
        }
        let extension = load_with(
            LoadOptions {
                allow_staged: Some(true),
                capabilities: Some(vec![
                    CAPABILITY_PANEL.to_string(),
                    ui_ext::v0_2::CAPABILITY_STATUS_BAR.to_string(),
                    ui_ext::v0_2::CAPABILITY_MODAL.to_string(),
                    "network:fetch".into(),
                ]),
                network_domains: Some(vec![format!("localhost:{}", server.port)]),
                capability_check: Some(Arc::new(AllowCalls)),
                ..LoadOptions::default()
            },
            cx,
        )
        .await
        .expect("loads");
        let url = format!("http://localhost:{}/hello", server.port);
        assert_eq!(
            command(&extension, &format!("fetch {url}")).await,
            ["denied"]
        );
        // No broker at all: the same.
        let no_broker = load_network(server.port, None, true, cx).await;
        assert_eq!(
            command(&no_broker, &format!("fetch {url}")).await,
            ["denied"]
        );
    }

    /// Decision 26: revoking cancels the requests in flight with `denied`, and the extension is
    /// told through `permission-changed`.
    #[gpui::test]
    async fn test_revoking_cancels_requests_in_flight(cx: &mut TestAppContext) {
        let server = server(cx);
        let broker = Broker::new(PermissionState::Granted, PermissionState::Granted);
        let extension = load_network(server.port, Some(broker.clone()), true, cx).await;
        let mut events = extension.take_io_events().expect("once");
        let hang = format!("http://localhost:{}/hang", server.port);
        assert_eq!(command(&extension, &format!("fetch {hang}")).await, ["1"]);
        // Let the request connect before revoking it.
        while server.connections() == 0 {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        broker.revoke();
        let mut outcomes = vec![
            deliver(&extension, &mut events).await,
            deliver(&extension, &mut events).await,
        ];
        outcomes.sort();
        assert_eq!(
            outcomes,
            [
                "fetch-done 1 err denied",
                "permission-changed T2Capability::NetworkFetch PermissionState::Denied"
            ]
        );
        // Later calls are refused at once.
        assert_eq!(
            command(&extension, &format!("fetch {hang}")).await,
            ["denied"]
        );
    }

    #[gpui::test]
    async fn test_cancel_caps_trap_and_reload(cx: &mut TestAppContext) {
        let server = server(cx);
        let broker = Broker::new(PermissionState::Granted, PermissionState::Granted);
        let extension = load_network(server.port, Some(broker.clone()), true, cx).await;
        let mut events = extension.take_io_events().expect("once");
        let hang = format!("http://localhost:{}/hang", server.port);

        // Cancel a running request.
        assert_eq!(command(&extension, &format!("fetch {hang}")).await, ["1"]);
        assert_eq!(command(&extension, "cancel 1").await, ["done"]);
        assert_eq!(
            deliver(&extension, &mut events).await,
            "fetch-done 1 err cancelled"
        );

        // At most 8 in flight, counted until delivered.
        let nine = [hang.as_str(); 9].join(" ");
        let results = command(&extension, &format!("fetch {nine}")).await;
        assert_eq!(results[..8], ["2", "3", "4", "5", "6", "7", "8", "9"]);
        assert_eq!(results[8], "limit-exceeded");

        // Reload: dropping the instance cancels its requests, and nothing is delivered.
        drop(extension);
        assert!(
            events.next().await.is_none(),
            "the old instance's outcomes are gone with it"
        );

        // A trap drops the request with the call's effects: it never starts.
        let extension = load_network(server.port, Some(broker), true, cx).await;
        let mut events = extension.take_io_events().expect("once");
        let before = server.connections();
        let error = extension
            .handle_event_v0_2(
                SURFACE,
                UiEvent::InputChanged(
                    1,
                    format!("fetch-trap http://localhost:{}/hello", server.port),
                ),
            )
            .await
            .unwrap_err();
        assert!(
            error.downcast_ref::<EffectReplyError>().is_none(),
            "{error:#}"
        );
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(
            events.try_recv().is_err(),
            "no outcome for a dropped request"
        );
        assert_eq!(server.connections(), before);
    }

    /// The manifest rules for `network:fetch` (design.md §0.6, §8).
    #[gpui::test]
    async fn test_network_manifest_rules(cx: &mut TestAppContext) {
        let fs = fs::FakeFs::new(cx.executor());
        fs.create_dir(Path::new("/ext/net")).await.expect("dir");
        let read = |fyx_ui: &str| {
            let fs = fs.clone();
            let text = format!(
                "id = \"net\"\nname = \"Net\"\nversion = \"0.1.0\"\nschema_version = 1\n[fyx_ui]\n\
                 component = \"fyx_ui.wasm\"\n{fyx_ui}"
            );
            async move {
                fs.insert_file("/ext/net/extension.toml", text.into_bytes())
                    .await;
                UiManifest::read(fs.as_ref(), Path::new("/ext/net")).await
            }
        };
        let manifest = read(
            "api_version = \"0.3.0\"\ncapabilities = [\"ui:panel\", \"network:fetch\"]\n\
             [fyx_ui.network]\ndomains = [\"api.example.com\", \"localhost:8080\"]\n",
        )
        .await
        .expect("valid")
        .expect("a fyx_ui table")
        .1;
        assert_eq!(
            manifest.network.expect("network").domains,
            ["api.example.com", "localhost:8080"]
        );
        for (fyx_ui, problem) in [
            (
                "api_version = \"0.2.0\"\ncapabilities = [\"network:fetch\"]\n\
                 [fyx_ui.network]\ndomains = [\"api.example.com\"]\n",
                "needs fyx:ui 0.3",
            ),
            (
                "api_version = \"0.3.0\"\ncapabilities = [\"network:fetch\"]\n",
                "needs [fyx_ui.network]",
            ),
            (
                "api_version = \"0.3.0\"\ncapabilities = [\"ui:panel\"]\n\
                 [fyx_ui.network]\ndomains = [\"api.example.com\"]\n",
                "needs \"network:fetch\"",
            ),
            (
                "api_version = \"0.3.0\"\ncapabilities = [\"network:fetch\"]\n\
                 [fyx_ui.network]\ndomains = [\"*.co.uk\"]\n",
                "public suffix",
            ),
            (
                "api_version = \"0.3.0\"\ncapabilities = [\"network:fetch\"]\n\
                 [fyx_ui.network]\ndomains = [\"api.example.com\"]\nextra = 1\n",
                "unknown field",
            ),
        ] {
            let error = read(fyx_ui).await.err().expect("refused");
            assert!(
                format!("{error:#}").contains(problem),
                "{problem}: {error:#}"
            );
        }
    }
}

/// integration-5 A4: the manifest is read version first, and each world declares only what it
/// can use (wit-0.3/migration.md §2).
#[gpui::test]
async fn test_manifests_are_read_version_first(cx: &mut TestAppContext) {
    let fs = fs::FakeFs::new(cx.executor());
    fs.create_dir(Path::new("/ext/m")).await.expect("dir");
    let read = |fyx_ui: &str| {
        let fs = fs.clone();
        let text = format!(
            "id = \"m\"\nname = \"M\"\nversion = \"0.1.0\"\nschema_version = 1\n[fyx_ui]\n\
             component = \"fyx_ui.wasm\"\n{fyx_ui}"
        );
        async move {
            fs.insert_file("/ext/m/extension.toml", text.into_bytes())
                .await;
            UiManifest::read(fs.as_ref(), Path::new("/ext/m")).await
        }
    };
    for fyx_ui in [
        "api_version = \"0.1.0\"\ncapabilities = [\"ui:panel\", \"ui:modal\"]\n",
        "api_version = \"0.2.0\"\ncapabilities = [\"ui:notifications\", \"ui:editor-decorations\"]\n",
        "api_version = \"0.3.0\"\ncapabilities = [\"ui:canvas\", \"ui:images\", \"network:fetch\"]\n[fyx_ui.network]\ndomains = [\"api.example.com\"]\n",
    ] {
        read(fyx_ui)
            .await
            .unwrap_or_else(|error| panic!("{fyx_ui}: {error:#}"));
    }
    for (fyx_ui, problem) in [
        // A newer world's tables do not hide its version.
        (
            "api_version = \"0.5.0\"\n[fyx_ui.future]\nx = 1\n",
            "targets fyx:ui 0.5.0",
        ),
        (
            "api_version = \"0.2.0\"\ncapabilities = [\"ui:canvas\"]\n",
            "\"ui:canvas\" needs fyx:ui 0.3",
        ),
        (
            "api_version = \"0.1.0\"\ncapabilities = [\"ui:notifications\"]\n",
            "\"ui:notifications\" needs fyx:ui 0.2",
        ),
        // The smaller 0.3 (integration-5 A3): the 0.4 capabilities and tables are refused.
        (
            "api_version = \"0.3.0\"\ncapabilities = [\"ui:picker\"]\n",
            "\"ui:picker\" is not in fyx:ui 0.3.0; it is planned for a later version",
        ),
        (
            "api_version = \"0.3.0\"\ncapabilities = [\"workspace:read-buffers\"]\n",
            "planned for a later version",
        ),
        (
            "api_version = \"0.2.0\"\ncapabilities = [\"ui:commands\"]\n",
            "planned for a later version",
        ),
        (
            "api_version = \"0.3.0\"\n[[fyx_ui.commands]]\nid = \"a\"\ntitle = \"A\"\n",
            "unknown field `commands`",
        ),
        (
            "api_version = \"0.3.0\"\n[[fyx_ui.picker_sources]]\nid = \"s\"\npicker = \"a/b\"\n",
            "unknown field `picker_sources`",
        ),
        (
            "api_version = \"0.3.0\"\ncapabilities = [\"ui:sparkles\"]\n",
            "unknown capability",
        ),
    ] {
        let error = read(fyx_ui).await.err().expect("refused");
        assert!(
            format!("{error:#}").contains(problem),
            "{problem}: {error:#}"
        );
    }
}

/// Plan 5.5 "done when": the 0.4 tables and capabilities load in a 0.4 manifest, a 0.3 manifest
/// with a 0.4 table or capability is refused, and `PATH`, `HOME`, `CLAUDE_CONFIG_DIR` and
/// `FOO_API_KEY` in `terminal:env` are refused.
#[gpui::test]
async fn test_v0_4_manifest_tables(cx: &mut TestAppContext) {
    let fs = fs::FakeFs::new(cx.executor());
    fs.create_dir(Path::new("/ext/t")).await.expect("dir");
    let read = |fyx_ui: &str| {
        let fs = fs.clone();
        let text = format!(
            "id = \"t\"\nname = \"T\"\nversion = \"0.1.0\"\nschema_version = 1\n[fyx_ui]\n\
             component = \"fyx_ui.wasm\"\n{fyx_ui}"
        );
        async move {
            fs.insert_file("/ext/t/extension.toml", text.into_bytes())
                .await;
            UiManifest::read(fs.as_ref(), Path::new("/ext/t")).await
        }
    };
    const LISTENER: &str = "[[fyx_ui.listeners]]\nname = \"ide\"\nprotocol = \"mcp-websocket\"\n\
         auth = { header = \"x-claude-code-ide-authorization\" }\nreason = \"lets Claude Code connect\"\n";
    let env = |vars: &str| {
        format!(
            "api_version = \"0.4.0\"\ncapabilities = [\"terminal:env\", \"network:listen\"]\n\
             {LISTENER}[fyx_ui.terminal_env]\nvars = {{ {vars} }}\nreason = \"connects the CLI\"\n"
        )
    };
    let claude = format!(
        "api_version = \"0.4.0\"\n\
         capabilities = [\"ui:panel\", \"terminal:profiles\", \"terminal:create\", \"terminal:env\", \
         \"network:listen\", \"fs:discovery-file\", \"relay:hooks\", \"package:files\", \
         \"review:propose\", \"context:editor\"]\n\
         [[fyx_ui.terminal_profiles]]\nid = \"claude\"\nname = \"Claude Code\"\nprogram = \"claude\"\n\
         args = [\"--ide\"]\naccount_tool = \"claude\"\n\
         [[fyx_ui.terminal_programs]]\nname = \"codex\"\nargs = [\"-c\", \"notify=*\", \"**\"]\n\
         reason = \"runs Codex in a panel\"\n\
         [fyx_ui.terminal_env]\nvars = {{ CLAUDE_CODE_SSE_PORT = \"${{listen.ide.port}}\", \
         ENABLE_IDE_INTEGRATION = \"true\", CLAUDE_CODE_PLUGIN_DIRS = \"${{package}}/claude-plugin\" }}\n\
         reason = \"connects Claude Code to Fyx\"\n\
         {LISTENER}\
         [[fyx_ui.discovery_files]]\nlistener = \"ide\"\ndir = \"${{account.claude}}/ide\"\n\
         default_dir = \"~/.claude/ide\"\nname = \"${{port}}.lock\"\n\
         content = {{ pid = \"${{pid}}\", authToken = \"${{token}}\", stdio = false, version = 2, \
         folders = [\"${{worktrees.joined}}\"], windows = \"${{platform.windows}}\", \
         ideInfo = {{ name = \"fyx\", displayName = \"Fyx\" }} }}\n\
         reason = \"Claude Code finds Fyx through it\"\n\
         [[fyx_ui.relay_channels]]\nname = \"pre-tool\"\non_failure = {{ exit = 0 }}\n\
         reason = \"the pre-tool hook\"\n"
    );
    let (_, manifest) = read(&claude)
        .await
        .unwrap_or_else(|error| panic!("{error:#}"))
        .expect("a fyx_ui table");
    assert_eq!(manifest.terminal_profiles[0].id, "claude");
    assert_eq!(manifest.listeners[0].name, "ide");
    assert_eq!(manifest.relay_channels[0].timeout_ms, 30_000);
    {
        use ui_ext::integration::DiscoveryValue;
        let content = &manifest.discovery_files[0].content;
        assert_eq!(content["stdio"], DiscoveryValue::Bool(false));
        assert_eq!(content["version"], DiscoveryValue::Integer(2));
        assert_eq!(
            content["folders"],
            DiscoveryValue::Array(vec!["${worktrees.joined}".into()])
        );
        assert_eq!(
            content["ideInfo"],
            DiscoveryValue::Table(
                [
                    ("name".to_string(), "fyx".into()),
                    ("displayName".to_string(), "Fyx".into()),
                ]
                .into()
            )
        );
    }
    read(&env("TOOL_IDE_PORT = \"${listen.ide.port}\""))
        .await
        .unwrap_or_else(|error| panic!("{error:#}"));
    // From 0.4 every T2 item says why (§C.2), processes and the network included.
    let (_, manifest) = read(
        "api_version = \"0.4.0\"\ncapabilities = [\"process:spawn\", \"network:fetch\"]\n\
         [[fyx_ui.processes]]\nname = \"git\"\nreason = \"shows the branch\"\n\
         [fyx_ui.network]\ndomains = [\"api.example.com\"]\nreason = \"checks for updates\"\n",
    )
    .await
    .unwrap_or_else(|error| panic!("{error:#}"))
    .expect("a fyx_ui table");
    assert_eq!(
        manifest.processes[0].reason.as_deref(),
        Some("shows the branch")
    );
    assert_eq!(
        manifest.network.unwrap().reason.as_deref(),
        Some("checks for updates")
    );
    // T3 (§C.3) is declared like any capability; the cards ask at the moment of action.
    let (_, manifest) = read(
        "api_version = \"0.4.0\"
capabilities = [\"terminal:read\", \"terminal:write\"]
",
    )
    .await
    .unwrap_or_else(|error| panic!("{error:#}"))
    .expect("a fyx_ui table");
    assert_eq!(manifest.capabilities, ["terminal:read", "terminal:write"]);

    let mut refused = vec![
        (
            "api_version = \"0.3.0\"\ncapabilities = [\"terminal:env\"]\n\
             [fyx_ui.terminal_env]\nvars = { A = \"b\" }\nreason = \"r\"\n"
                .to_string(),
            "\"terminal:env\" needs fyx:ui 0.4".to_string(),
        ),
        (
            format!("api_version = \"0.3.0\"\ncapabilities = [\"network:listen\"]\n{LISTENER}"),
            "needs fyx:ui 0.4".to_string(),
        ),
        (
            "api_version = \"0.3.0\"\n[[fyx_ui.terminal_profiles]]\nid = \"a\"\nname = \"A\"\nprogram = \"a\"\n"
                .to_string(),
            "[[fyx_ui.terminal_profiles]] needs fyx:ui 0.4".to_string(),
        ),
        (
            "api_version = \"0.4.0\"\ncapabilities = [\"terminal:create\"]\n".to_string(),
            "terminal:create needs [[fyx_ui.terminal_programs]]".to_string(),
        ),
        (
            "api_version = \"0.4.0\"\n[[fyx_ui.terminal_programs]]\nname = \"codex\"\nreason = \"r\"\n"
                .to_string(),
            "needs the terminal:create capability".to_string(),
        ),
        (
            "api_version = \"0.4.0\"\ncapabilities = [\"terminal:create\"]\n\
             [[fyx_ui.terminal_programs]]\nname = \"codex\"\n"
                .to_string(),
            "missing field `reason`".to_string(),
        ),
        (
            env("PLUGINS = \"${package}/plugins\""),
            "${package} in [fyx_ui.terminal_env] needs the package:files capability".to_string(),
        ),
        (
            env("TOOL_PORT = \"${listen.other.port}\""),
            "no listener named".to_string(),
        ),
        (
            "api_version = \"0.4.0\"\ncapabilities = [\"process:spawn\"]\n\
             [[fyx_ui.processes]]\nname = \"git\"\n"
                .to_string(),
            "[[fyx_ui.processes]] \"git\": a reason is required".to_string(),
        ),
        (
            "api_version = \"0.4.0\"\ncapabilities = [\"network:fetch\"]\n\
             [fyx_ui.network]\ndomains = [\"a.example.com\"]\nreason = \"a\\tb\"\n"
                .to_string(),
            "the reason must be one line".to_string(),
        ),
        (
            "api_version = \"0.3.0\"\ncapabilities = [\"network:fetch\"]\n\
             [fyx_ui.network]\ndomains = [\"a.example.com\"]\nreason = \"r\"\n"
                .to_string(),
            "[fyx_ui.network]: reason needs fyx:ui 0.4".to_string(),
        ),
    ];
    for name in [
        "PATH",
        "HOME",
        "CLAUDE_CONFIG_DIR",
        "FOO_API_KEY",
        "path",
        "LD_PRELOAD",
    ] {
        refused.push((
            env(&format!("{name} = \"x\"")),
            format!("terminal:env name {name}"),
        ));
    }
    for (fyx_ui, problem) in refused {
        let error = read(&fyx_ui).await.err().expect("refused");
        assert!(
            format!("{error:#}").contains(&problem),
            "{problem}: {error:#}"
        );
    }
}

/// `process:spawn` end to end (design.md §9, plan 4.7): spec §4 failure 4, the tree dies on
/// reload, the caps, kill, stdin, the filtered environment, consent and the worktree rules. The
/// rules alone are tested in `process.rs` and `ui_ext::process`.
mod process_spawn {
    use std::{
        collections::HashMap,
        ffi::OsString,
        path::PathBuf,
        sync::Mutex,
        time::{Duration, Instant},
    };

    use super::*;
    use crate::{
        network::IoEvent,
        permissions::{CapabilityCheck, CapabilityDenied, PermissionState},
        process::{Environment, ProcessSpec, Workspace, WorktreeRoot},
    };

    struct Broker {
        state: Mutex<PermissionState>,
        answer: PermissionState,
        subscribers: Mutex<Vec<futures::channel::mpsc::UnboundedSender<(String, PermissionState)>>>,
    }

    impl Broker {
        fn new(state: PermissionState, answer: PermissionState) -> Arc<Self> {
            Arc::new(Self {
                state: Mutex::new(state),
                answer,
                subscribers: Mutex::new(Vec::new()),
            })
        }

        fn revoke(&self) {
            *self.state.lock().unwrap() = PermissionState::Denied;
            for subscriber in self.subscribers.lock().unwrap().iter() {
                subscriber
                    .unbounded_send(("process:spawn".into(), PermissionState::Denied))
                    .ok();
            }
        }
    }

    impl CapabilityCheck for Broker {
        fn check(&self, _: &str, _: &str) -> Result<(), CapabilityDenied> {
            Ok(())
        }

        fn state(&self, _: &str, capability: &str) -> PermissionState {
            if capability == "process:spawn" {
                *self.state.lock().unwrap()
            } else {
                PermissionState::Unavailable
            }
        }

        fn request(
            &self,
            _: &str,
            _: &str,
        ) -> futures::future::BoxFuture<'static, PermissionState> {
            *self.state.lock().unwrap() = self.answer;
            futures::future::ready(self.answer).boxed()
        }

        fn changes(
            &self,
            _: &str,
        ) -> futures::channel::mpsc::UnboundedReceiver<(String, PermissionState)> {
            let (sender, receiver) = futures::channel::mpsc::unbounded();
            self.subscribers.lock().unwrap().push(sender);
            receiver
        }
    }

    struct Worktrees(Vec<WorktreeRoot>);

    impl Workspace for Worktrees {
        fn worktrees(&self) -> Vec<WorktreeRoot> {
            self.0.clone()
        }
    }

    fn spec(name: &str) -> ProcessSpec {
        ProcessSpec {
            name: name.into(),
            args: vec!["**".into()],
            path: None,
            reason: None,
        }
    }

    struct Setup {
        broker: Arc<Broker>,
        worktree: tempfile::TempDir,
        trusted: bool,
        programs: Vec<ProcessSpec>,
        env: Vec<String>,
        environment: Option<Environment>,
    }

    impl Setup {
        fn new(programs: &[&str]) -> Self {
            Self {
                broker: Broker::new(PermissionState::Granted, PermissionState::Granted),
                worktree: tempfile::tempdir().expect("a worktree"),
                trusted: true,
                programs: programs.iter().map(|name| spec(name)).collect(),
                env: Vec::new(),
                environment: None,
            }
        }

        async fn load(&self, cx: &mut TestAppContext) -> UiExtension {
            load_with(
                LoadOptions {
                    allow_staged: Some(true),
                    capabilities: Some(vec![
                        CAPABILITY_PANEL.to_string(),
                        ui_ext::v0_2::CAPABILITY_STATUS_BAR.to_string(),
                        ui_ext::v0_2::CAPABILITY_MODAL.to_string(),
                        "process:spawn".into(),
                    ]),
                    processes: Some(self.programs.clone()),
                    process_env: Some(self.env.clone()),
                    capability_check: Some(self.broker.clone()),
                    workspace: Some(Arc::new(Worktrees(vec![WorktreeRoot {
                        id: 1,
                        root: self.worktree.path().to_path_buf(),
                        trusted: self.trusted,
                    }]))),
                    process_environment: self.environment.clone(),
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

    /// Delivers outcomes until `id` exits or fails; returns its stdout and the final line.
    async fn run_to_end(
        extension: &UiExtension,
        events: &mut futures::channel::mpsc::Receiver<IoEvent>,
        id: &str,
    ) -> (String, String) {
        let mut stdout = String::new();
        loop {
            let event = events.next().await.expect("an outcome");
            let reply = extension.handle_io(event).await.expect("handle-io");
            let Some(HostEffect::Push {
                update: Update::Replace(nodes),
                ..
            }) = reply.effects.first()
            else {
                continue;
            };
            let Props::Text(text) = &nodes[0].props else {
                continue;
            };
            let line = text.content.clone();
            if let Some(rest) = line.strip_prefix(&format!("output {id} stdout ")) {
                stdout.push_str(rest);
            } else if line.starts_with(&format!("exited {id} "))
                || line.starts_with(&format!("spawn-failed {id} "))
            {
                return (stdout, line);
            }
        }
    }

    #[cfg(windows)]
    fn system32() -> PathBuf {
        PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot")).join("System32")
    }

    /// Spec §4 failure 4, end to end: the worktree plants `hostname`, and the worktree and `.`
    /// come first on PATH. The system program runs.
    // The reference `hostname` runs blocking, in the test thread.
    #[allow(clippy::disallowed_methods)]
    #[gpui::test]
    async fn test_failure_4_the_planted_program_never_runs(cx: &mut TestAppContext) {
        let mut setup = Setup::new(&["hostname"]);
        #[cfg(windows)]
        let (system, planted) = {
            std::fs::copy(
                system32().join("whoami.exe"),
                setup.worktree.path().join("hostname.exe"),
            )
            .expect("plants");
            (system32(), "whoami")
        };
        #[cfg(unix)]
        let (system, planted) = {
            use std::os::unix::fs::PermissionsExt as _;
            let path = setup.worktree.path().join("hostname");
            std::fs::write(&path, "#!/bin/sh\necho planted\n").expect("plants");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
            (PathBuf::from("/bin"), "planted")
        };
        let path = std::env::join_paths([
            setup.worktree.path().to_path_buf(),
            PathBuf::from("."),
            system,
            PathBuf::from("/usr/bin"),
        ])
        .expect("PATH");
        let mut base: Vec<(OsString, OsString)> = std::env::vars_os()
            .filter(|(name, _)| !name.eq_ignore_ascii_case("PATH"))
            .collect();
        base.push(("PATH".into(), path));
        setup.environment = Some(Arc::new(move || base.clone()));
        let extension = setup.load(cx).await;
        let mut events = extension.take_io_events().expect("once");
        let id = command(&extension, "spawn -- hostname").await;
        assert_eq!(id, "1");
        let (stdout, end) = run_to_end(&extension, &mut events, &id).await;
        assert_eq!(end, "exited 1 0 false");
        let real = std::process::Command::new("hostname")
            .output()
            .expect("runs");
        assert_eq!(stdout.trim(), String::from_utf8_lossy(&real.stdout).trim());
        assert!(
            !stdout.contains(planted) || stdout.trim() == planted,
            "{stdout}"
        );
    }

    /// Plan 4.7, done: a reload kills the whole tree, grandchildren included.
    #[gpui::test]
    async fn test_reload_kills_the_process_tree(cx: &mut TestAppContext) {
        #[cfg(windows)]
        let (program, script) = (
            "powershell",
            "-NoProfile -Command $p=Start-Process_ping_-ArgumentList_'-n','60','127.0.0.1'_-PassThru_-WindowStyle_Hidden;Write-Output_$p.Id;Start-Sleep_60",
        );
        #[cfg(unix)]
        let (program, script) = ("sh", "-c sleep_60_&_echo_$!;_sleep_60");
        let setup = Setup::new(&[program]);
        let extension = setup.load(cx).await;
        let mut events = extension.take_io_events().expect("once");
        let id = command(&extension, &format!("spawn -- {program} {script}")).await;
        assert_eq!(id, "1");
        // The grandchild's pid, from its parent's first line.
        let mut stdout = String::new();
        let grandchild: u32 = loop {
            let event = events.next().await.expect("an outcome");
            if let IoEvent::ProcessOutput(output) = &event {
                stdout.push_str(&String::from_utf8_lossy(&output.data));
            }
            extension.handle_io(event).await.expect("handle-io");
            if let Some(line) = stdout.lines().next().filter(|_| stdout.contains('\n'))
                && let Ok(pid) = line.trim().parse()
            {
                break pid;
            }
        };
        assert!(
            is_running(grandchild),
            "the grandchild runs before the reload"
        );
        drop(extension);
        let deadline = Instant::now() + Duration::from_secs(10);
        while is_running(grandchild) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(
            !is_running(grandchild),
            "the grandchild died with the instance"
        );
    }

    // A blocking `tasklist`, in the test thread.
    #[cfg(windows)]
    #[allow(clippy::disallowed_methods)]
    fn is_running(pid: u32) -> bool {
        let output = std::process::Command::new(system32().join("tasklist.exe"))
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .expect("tasklist");
        String::from_utf8_lossy(&output.stdout).contains(&pid.to_string())
    }

    #[cfg(unix)]
    fn is_running(pid: u32) -> bool {
        // SAFETY: signal 0 only checks that the process exists.
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }

    #[gpui::test]
    async fn test_kill_caps_and_not_found(cx: &mut TestAppContext) {
        #[cfg(windows)]
        let (sleeper, sleep) = ("ping", "-n 60 127.0.0.1");
        #[cfg(unix)]
        let (sleeper, sleep) = ("sleep", "60");
        let mut setup = Setup::new(&[sleeper]);
        setup.programs.push(spec("tool.cmd"));
        let extension = setup.load(cx).await;
        let mut events = extension.take_io_events().expect("once");
        // Four at most, counted until they exit.
        let mut ids = Vec::new();
        for _ in 0..4 {
            ids.push(command(&extension, &format!("spawn -- {sleeper} {sleep}")).await);
        }
        assert_eq!(ids, ["1", "2", "3", "4"]);
        assert_eq!(
            command(&extension, &format!("spawn -- {sleeper} {sleep}")).await,
            "limit-exceeded"
        );
        for id in &ids {
            assert_eq!(command(&extension, &format!("kill {id}")).await, "done");
        }
        let mut exited = 0;
        while exited < 4 {
            let event = events.next().await.expect("an outcome");
            if let IoEvent::ProcessExited(exit) = &event {
                assert!(exit.killed);
                exited += 1;
            }
            extension.handle_io(event).await.expect("handle-io");
        }
        assert_eq!(
            command(&extension, "kill 1").await,
            "invalid-argument",
            "exited"
        );
        // Undeclared, and a batch script: not found, without starting anything.
        let undeclared = command(&extension, "spawn -- rm -rf").await;
        assert_eq!(undeclared, "5");
        let (_, end) = run_to_end(&extension, &mut events, &undeclared).await;
        assert_eq!(end, "spawn-failed 5 not-found");
        let script = command(&extension, "spawn -- tool.cmd").await;
        let (_, end) = run_to_end(&extension, &mut events, &script).await;
        assert_eq!(end, format!("spawn-failed {script} not-found"));
    }

    #[gpui::test]
    async fn test_kill_reports_killed(cx: &mut TestAppContext) {
        #[cfg(windows)]
        let (sleeper, sleep) = ("ping", "-n 60 127.0.0.1");
        #[cfg(unix)]
        let (sleeper, sleep) = ("sleep", "60");
        let setup = Setup::new(&[sleeper]);
        let extension = setup.load(cx).await;
        let mut events = extension.take_io_events().expect("once");
        let id = command(&extension, &format!("spawn -- {sleeper} {sleep}")).await;
        // Let it start before killing it.
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(command(&extension, &format!("kill {id}")).await, "done");
        let (_, end) = run_to_end(&extension, &mut events, &id).await;
        assert_eq!(end, format!("exited {id} none true"));
        let record = extension
            .diagnostics()
            .io
            .back()
            .cloned()
            .expect("a record");
        assert_eq!(record.capability, "process:spawn");
        assert_eq!(
            record.what,
            format!("{sleeper} ({} arguments)", sleep.split(' ').count())
        );
        assert_eq!(record.outcome.as_deref(), Some("killed"));
    }

    /// Revoking `process:spawn` kills running processes; a refused prompt never starts one.
    #[gpui::test]
    async fn test_consent_and_revocation(cx: &mut TestAppContext) {
        #[cfg(windows)]
        let (sleeper, sleep) = ("ping", "-n 60 127.0.0.1");
        #[cfg(unix)]
        let (sleeper, sleep) = ("sleep", "60");
        let setup = Setup::new(&[sleeper]);
        let extension = setup.load(cx).await;
        let mut events = extension.take_io_events().expect("once");
        let id = command(&extension, &format!("spawn -- {sleeper} {sleep}")).await;
        std::thread::sleep(Duration::from_millis(300));
        setup.broker.revoke();
        let mut outcomes = HashMap::new();
        while outcomes.len() < 2 {
            let event = events.next().await.expect("an outcome");
            let kind = match &event {
                IoEvent::ProcessExited(exit) => Some(("exited", exit.killed)),
                IoEvent::PermissionChanged(..) => Some(("permission", true)),
                _ => None,
            };
            extension.handle_io(event).await.expect("handle-io");
            if let Some((kind, killed)) = kind {
                outcomes.insert(kind, killed);
            }
        }
        assert_eq!(outcomes.get("exited"), Some(&true), "killed on revoke");
        assert_eq!(
            command(&extension, &format!("spawn -- {sleeper} {sleep}")).await,
            "denied"
        );
        let _ = id;

        let mut refused = Setup::new(&[sleeper]);
        refused.broker = Broker::new(PermissionState::Ask, PermissionState::Denied);
        let extension = refused.load(cx).await;
        let mut events = extension.take_io_events().expect("once");
        let id = command(&extension, &format!("spawn -- {sleeper} {sleep}")).await;
        let (_, end) = run_to_end(&extension, &mut events, &id).await;
        assert_eq!(end, format!("spawn-failed {id} denied"));
    }

    /// Decision 26: an untrusted worktree is refused without a prompt; integration-5 A5 gives it
    /// and "no worktree" their own codes, so an extension need not match the message.
    #[gpui::test]
    async fn test_worktree_rules(cx: &mut TestAppContext) {
        let mut setup = Setup::new(&["hostname"]);
        setup.trusted = false;
        let extension = setup.load(cx).await;
        assert_eq!(
            command(&extension, "spawn -- hostname").await,
            "untrusted-worktree"
        );
        let none = load_with(
            LoadOptions {
                allow_staged: Some(true),
                capabilities: Some(vec![
                    CAPABILITY_PANEL.to_string(),
                    ui_ext::v0_2::CAPABILITY_STATUS_BAR.to_string(),
                    ui_ext::v0_2::CAPABILITY_MODAL.to_string(),
                    "process:spawn".into(),
                ]),
                processes: Some(vec![spec("hostname")]),
                capability_check: Some(setup.broker.clone()),
                ..LoadOptions::default()
            },
            cx,
        )
        .await
        .expect("loads");
        assert_eq!(command(&none, "spawn -- hostname").await, "no-worktree");
    }

    /// The child sees only the filtered environment, and stdin reaches it.
    #[gpui::test]
    async fn test_environment_and_stdin(cx: &mut TestAppContext) {
        #[cfg(windows)]
        let (env_program, env_args, cat) = ("cmd", "/c set", "sort");
        #[cfg(unix)]
        let (env_program, env_args, cat) = ("env", "", "cat");
        let mut setup = Setup::new(&[env_program, cat]);
        setup.env = vec!["FORCE_COLOR".into(), "COLUMNS".into()];
        let mut base: Vec<(OsString, OsString)> = std::env::vars_os().collect();
        base.push(("FORCE_COLOR".into(), "from-fyx".into()));
        base.push(("FYX_SECRET".into(), "never".into()));
        setup.environment = Some(Arc::new(move || base.clone()));
        let extension = setup.load(cx).await;
        let mut events = extension.take_io_events().expect("once");
        assert_eq!(
            command(
                &extension,
                &format!("spawn env FYX_SECRET=x -- {env_program} {env_args}")
            )
            .await,
            "invalid-argument",
            "an undeclared name"
        );
        let id = command(
            &extension,
            &format!("spawn env COLUMNS=from-guest -- {env_program} {env_args}"),
        )
        .await;
        let (stdout, end) = run_to_end(&extension, &mut events, &id).await;
        assert_eq!(end, format!("exited {id} 0 false"));
        assert!(stdout.contains("FORCE_COLOR=from-fyx"), "{stdout}");
        assert!(stdout.contains("COLUMNS=from-guest"), "{stdout}");
        assert!(!stdout.contains("FYX_SECRET"), "{stdout}");

        let id = command(&extension, &format!("spawn stdin -- {cat}")).await;
        assert_eq!(
            command(&extension, &format!("write {id} beta")).await,
            "done"
        );
        assert_eq!(
            command(&extension, &format!("write {id} alpha")).await,
            "done"
        );
        assert_eq!(command(&extension, &format!("close {id}")).await, "done");
        let (stdout, end) = run_to_end(&extension, &mut events, &id).await;
        assert_eq!(end, format!("exited {id} 0 false"));
        let lines: Vec<&str> = stdout.lines().map(str::trim).collect();
        #[cfg(windows)]
        assert_eq!(lines, ["alpha", "beta"]);
        #[cfg(unix)]
        assert_eq!(lines, ["beta", "alpha"]);
    }
}

/// The harness (`fyx_ext_harness`) against the host on the same 0.3 probe: every case's effects,
/// in `Debug` form, and its reply must match (plan 4.12).
mod harness_conformance_v0_3 {
    use fyx_ext_harness::Harness;

    use super::*;

    fn debug<T: std::fmt::Debug>(values: &[T]) -> Vec<String> {
        values.iter().map(|value| format!("{value:?}")).collect()
    }

    fn harness() -> Harness {
        Harness::load(&probe_v0_3_wasm()).expect("the harness loads the 0.3 probe")
    }

    /// Sends `event` to `surface_id` in both and checks that the effects and the reply match.
    async fn compare(host: &UiExtension, harness: &mut Harness, surface_id: &str, event: UiEvent) {
        let (host_effects, host_value) =
            match host.handle_event_v0_2(surface_id, event.clone()).await {
                Ok(reply) => (reply.effects, Ok(reply.value)),
                Err(error) => {
                    let error = error
                        .downcast_ref::<EffectReplyError>()
                        .unwrap_or_else(|| panic!("{event:?}: not a guest error: {error:#}"));
                    (error.effects.clone(), Err(()))
                }
            };
        let reply = harness
            .handle_event_raw_v0_2(surface_id, event.clone())
            .unwrap_or_else(|error| panic!("{event:?}: the harness trapped: {error:#}"));
        assert_eq!(debug(&host_effects), debug(&reply.effects), "{event:?}");
        assert_eq!(
            host_value.map(|value| format!("{value:?}")),
            reply
                .value
                .map(|value| format!("{value:?}"))
                .map_err(|_| ()),
            "{surface_id} {event:?}"
        );
    }

    /// A load without a manifest grants the same capabilities in both.
    #[test]
    fn test_all_capabilities_match_the_harness() {
        let mut host = crate::ALL_UI_CAPABILITIES.to_vec();
        let mut harness = fyx_ext_harness::ALL_CAPABILITIES.to_vec();
        host.sort_unstable();
        harness.sort_unstable();
        assert_eq!(host, harness);
    }

    #[gpui::test]
    async fn test_images_match_the_harness(cx: &mut TestAppContext) {
        let host = load(cx).await;
        let mut harness = harness();
        assert_eq!(harness.api_version(), &Version::new(0, 3, 0));
        assert_eq!(debug(host.surfaces_v0_2()), debug(harness.surfaces_v0_2()));
        // add-image, its refusals, the 32-image cap and the checks of image pushes, in one
        // instance so the handles advance alike.
        for case in 1..=4 {
            compare(&host, &mut harness, SURFACE, UiEvent::Click(case)).await;
        }
    }

    /// Canvases (plan 4.13, 4.15): the checks of a push (caps, NaN, placement), replies on each
    /// surface kind, and a `canvas-click`, alike in the host and the harness.
    #[gpui::test]
    async fn test_canvases_match_the_harness(cx: &mut TestAppContext) {
        let host = load(cx).await;
        let mut harness = harness();
        compare(&host, &mut harness, SURFACE, UiEvent::Click(8)).await;
        for surface in ["status", "modal", SURFACE] {
            compare(&host, &mut harness, surface, UiEvent::Click(9)).await;
        }
        let click = UiEvent::CanvasClick(ui_ext::canvas::CanvasClick {
            canvas: 1,
            x: 12.5.into(),
            y: 3.0.into(),
        });
        compare(&host, &mut harness, SURFACE, click).await;
        assert!(harness.disabled().is_none());
    }

    /// `permissions` (probe cases 10 and 11): undeclared capabilities query `unavailable` and
    /// refuse `request`, alike.
    #[gpui::test]
    async fn test_permissions_match_the_harness(cx: &mut TestAppContext) {
        let host = load(cx).await;
        let mut harness = harness();
        for case in [10, 11] {
            compare(&host, &mut harness, SURFACE, UiEvent::Click(case)).await;
        }
    }

    #[gpui::test]
    async fn test_image_replies_match_the_harness(cx: &mut TestAppContext) {
        let host = load(cx).await;
        let mut harness = harness();
        for surface in [SURFACE, "status", "modal"] {
            compare(&host, &mut harness, surface, UiEvent::Click(5)).await;
            let host_rows = host.list_rows_v0_2(surface, 7, 0, 1).await;
            let harness_rows = harness.rows_v0_2(surface, 7, 0, 1);
            assert_eq!(host_rows.is_ok(), harness_rows.is_ok(), "{surface}");
            if let (Ok(host_rows), Ok(harness_rows)) = (host_rows, harness_rows) {
                assert_eq!(debug(&host_rows.value), debug(&harness_rows));
            }
        }
    }

    #[gpui::test]
    async fn test_the_capability_matches_the_harness(cx: &mut TestAppContext) {
        let capabilities = [
            CAPABILITY_PANEL,
            ui_ext::v0_2::CAPABILITY_STATUS_BAR,
            ui_ext::v0_2::CAPABILITY_MODAL,
        ];
        let host = load_with(
            LoadOptions {
                capabilities: Some(capabilities.iter().map(|&name| name.into()).collect()),
                allow_staged: Some(true),
                ..LoadOptions::default()
            },
            cx,
        )
        .await
        .expect("loads");
        let mut harness = Harness::load_with_capabilities(&probe_v0_3_wasm(), &capabilities)
            .expect("the harness loads the 0.3 probe");
        compare(&host, &mut harness, SURFACE, UiEvent::Click(1)).await;
    }

    /// 0.3 keeps 0.2's `place-decoration` in the harness too (integration-5 A3).
    #[gpui::test]
    async fn test_place_decoration_matches_the_harness(cx: &mut TestAppContext) {
        let host = load(cx).await;
        let mut harness = harness();
        compare(&host, &mut harness, SURFACE, UiEvent::Click(6)).await;
    }
}

/// `network:fetch` (plan 4.6) in the host and the harness: the import's checks and ids, the
/// default without a broker (`denied`), and delivery through `handle-io` with a granted broker.
mod harness_conformance_fetch {
    use fyx_ext_harness::{Harness, TestBroker};

    use super::*;
    use crate::{
        network::test_server,
        permissions::{CapabilityCheck, CapabilityDenied, PermissionState},
    };

    struct GrantAll;

    impl CapabilityCheck for GrantAll {
        fn check(&self, _: &str, _: &str) -> Result<(), CapabilityDenied> {
            Ok(())
        }

        fn state(&self, _: &str, _: &str) -> PermissionState {
            PermissionState::Granted
        }
    }

    fn debug<T: std::fmt::Debug>(values: &[T]) -> Vec<String> {
        values.iter().map(|value| format!("{value:?}")).collect()
    }

    fn capabilities() -> Vec<String> {
        vec![
            CAPABILITY_PANEL.into(),
            ui_ext::v0_2::CAPABILITY_STATUS_BAR.into(),
            ui_ext::v0_2::CAPABILITY_MODAL.into(),
            "network:fetch".into(),
        ]
    }

    async fn both(
        port: u16,
        broker: Option<Arc<dyn CapabilityCheck>>,
        cx: &mut TestAppContext,
    ) -> (UiExtension, Harness) {
        let domain = format!("localhost:{port}");
        let host = load_with(
            LoadOptions {
                allow_staged: Some(true),
                capabilities: Some(capabilities()),
                network_domains: Some(vec![domain.clone()]),
                capability_check: broker,
                ..LoadOptions::default()
            },
            cx,
        )
        .await
        .expect("loads");
        let capabilities = capabilities();
        let capabilities = capabilities.iter().map(String::as_str).collect::<Vec<_>>();
        let harness = Harness::load_with_capabilities(&probe_v0_3_wasm(), &capabilities)
            .and_then(|harness| harness.with_network_domains(&[&domain]))
            .expect("the harness loads the 0.3 probe");
        (host, harness)
    }

    /// Runs probe command `command` in both; the results and effects must match.
    async fn command(host: &UiExtension, harness: &mut Harness, command: &str) -> String {
        let event = UiEvent::InputChanged(1, command.into());
        let reply = host
            .handle_event_v0_2(SURFACE, event.clone())
            .await
            .unwrap_or_else(|error| panic!("{command}: {error:#}"));
        let raw = harness
            .handle_event_raw(SURFACE, event)
            .unwrap_or_else(|error| panic!("{command}: the harness trapped: {error:#}"));
        assert_eq!(debug(&reply.effects), debug(&raw.effects), "{command}");
        assert_eq!(
            format!("{:?}", reply.value),
            format!("{:?}", raw.value.expect("a report")),
            "{command}"
        );
        format!("{:?}", reply.value)
    }

    /// The harness's own broker paths: a prompt answered `denied`, and a cancelled request.
    #[test]
    fn test_the_harness_prompts_and_cancels() {
        let capabilities = capabilities();
        let capabilities = capabilities.iter().map(String::as_str).collect::<Vec<_>>();
        let mut harness = Harness::load_with_capabilities(&probe_v0_3_wasm(), &capabilities)
            .and_then(|harness| harness.with_network_domains(&["localhost:8080"]))
            .expect("loads");
        harness.set_broker(TestBroker::asking(
            PermissionState::Denied,
            Box::new(|_| panic!("a refused request never reaches the network")),
        ));
        let report = |harness: &mut Harness, command: &str| {
            format!(
                "{:?}",
                harness
                    .handle_event_raw(SURFACE, UiEvent::InputChanged(1, command.into()))
                    .expect("no trap")
                    .value
            )
        };
        assert!(report(&mut harness, "fetch http://localhost:8080/a").contains("\"1\""));
        let delivered = harness.deliver_io().expect("deliver");
        assert!(format!("{:?}", delivered[0].effects).contains("fetch-done 1 err denied"));
        assert_eq!(harness.broker().expect("broker").prompts(), 1);
        // Denied now: the next fetch is refused at the import.
        assert!(report(&mut harness, "fetch http://localhost:8080/a").contains("denied"));

        harness.set_broker(TestBroker::granted(Box::new(|_| {
            panic!("a cancelled request never reaches the network")
        })));
        report(&mut harness, "fetch http://localhost:8080/b");
        report(&mut harness, "cancel 2");
        let delivered = harness.deliver_io().expect("deliver");
        assert!(
            format!("{:?}", delivered[0].effects).contains("fetch-done 2 err cancelled"),
            "{:?}",
            delivered[0].effects
        );

        // A call that traps drops its requests without an outcome, as in the host.
        assert!(
            harness
                .handle_event_raw(
                    SURFACE,
                    UiEvent::InputChanged(1, "fetch-trap http://localhost:8080/c".into())
                )
                .is_err()
        );
        assert!(harness.pending_fetches().is_empty());
    }

    #[gpui::test]
    async fn test_without_a_broker_fetch_is_denied_alike(cx: &mut TestAppContext) {
        let (host, mut harness) = both(8080, None, cx).await;
        let report = command(&host, &mut harness, "fetch http://localhost:8080/hello").await;
        assert!(report.contains("denied"), "{report}");
        assert!(harness.pending_fetches().is_empty());
    }

    #[gpui::test]
    async fn test_granted_fetches_match_the_harness(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let server = test_server::start_in_thread(|path, _| match path {
            "/hello" => Some(test_server::ok(b"hello")),
            _ => None,
        });
        let (host, mut harness) = both(server.port, Some(Arc::new(GrantAll)), cx).await;
        let url = format!("http://localhost:{}", server.port);
        harness.set_broker(TestBroker::granted(Box::new(move |request| {
            assert!(request.url.ends_with("/hello"), "{}", request.url);
            Ok(ui_ext::network::FetchResponse {
                status: 200,
                headers: Vec::new(),
                body: b"hello".to_vec(),
                url: request.url.clone(),
            })
        })));
        let mut events = host.take_io_events().expect("once");

        // The import's checks: an undeclared host, a bad URL and the scheme, then two accepted.
        let report = command(
            &host,
            &mut harness,
            &format!(
                "fetch https://example.com/x ftp://localhost/x not-a-url {url}/hello {url}/hello"
            ),
        )
        .await;
        assert!(report.contains("1 | 2"), "{report}");
        assert_eq!(harness.pending_fetches().len(), 2);

        // permissions: the declared capability's state, and a request without a gesture.
        let report = command(&host, &mut harness, "perm query network-fetch").await;
        assert!(report.contains("granted"), "{report}");
        let report = command(&host, &mut harness, "perm query process-spawn").await;
        assert!(report.contains("unavailable"), "{report}");
        let report = command(&host, &mut harness, "perm request network-fetch").await;
        assert!(report.contains("not-permitted"), "{report}");

        // The same two outcomes through handle-io, with the same pushes. The host's requests
        // run concurrently, so its order may differ from the harness's, which is by id.
        let delivered = harness.deliver_io().expect("deliver");
        assert_eq!(delivered.len(), 2);
        let mut harness_effects = delivered
            .iter()
            .map(|delivered| debug(&delivered.effects))
            .collect::<Vec<_>>();
        let mut host_effects = Vec::new();
        for _ in 0..2 {
            let event = events.next().await.expect("an outcome");
            let reply = host.handle_io(event).await.expect("handle-io");
            host_effects.push(debug(&reply.effects));
        }
        harness_effects.sort();
        host_effects.sort();
        assert_eq!(host_effects, harness_effects);
        assert!(
            host_effects
                .iter()
                .flatten()
                .all(|effect| effect.contains("200 hello")),
            "{host_effects:?}"
        );
        assert!(harness.pending_fetches().is_empty());
    }
}

/// `process:spawn` (plan 4.7) in the host and the harness: the import's checks, ids and caps,
/// and the outcome of a program that cannot start. The programs are declared but do not exist,
/// so the host runs nothing; the harness runs no real processes at all.
mod harness_conformance_process {
    use fyx_ext_harness::{FakeRun, Harness, TestBroker};

    use super::*;
    use crate::{
        permissions::{CapabilityCheck, CapabilityDenied, PermissionState},
        process::{ProcessSpec, Workspace, WorktreeRoot},
    };

    const MISSING: &str = "fyx-harness-no-such-program";

    struct GrantAll;

    impl CapabilityCheck for GrantAll {
        fn check(&self, _: &str, _: &str) -> Result<(), CapabilityDenied> {
            Ok(())
        }

        fn state(&self, _: &str, _: &str) -> PermissionState {
            PermissionState::Granted
        }
    }

    struct Worktrees(Vec<WorktreeRoot>);

    impl Workspace for Worktrees {
        fn worktrees(&self) -> Vec<WorktreeRoot> {
            self.0.clone()
        }
    }

    fn debug<T: std::fmt::Debug>(values: &[T]) -> Vec<String> {
        values.iter().map(|value| format!("{value:?}")).collect()
    }

    fn programs() -> Vec<ProcessSpec> {
        vec![
            ProcessSpec {
                name: MISSING.into(),
                args: vec!["status".into(), "**".into()],
                path: None,
                reason: None,
            },
            // Any arguments: without patterns a program runs with none (integration-5 S6).
            ProcessSpec {
                name: format!("{MISSING}-2"),
                args: vec!["**".into()],
                path: None,
                reason: None,
            },
        ]
    }

    fn capabilities() -> Vec<String> {
        vec![
            CAPABILITY_PANEL.into(),
            ui_ext::v0_2::CAPABILITY_STATUS_BAR.into(),
            ui_ext::v0_2::CAPABILITY_MODAL.into(),
            "process:spawn".into(),
        ]
    }

    async fn both(
        broker: Option<Arc<dyn CapabilityCheck>>,
        worktree: Option<&tempfile::TempDir>,
        cx: &mut TestAppContext,
    ) -> (UiExtension, Harness) {
        let host = load_with(
            LoadOptions {
                allow_staged: Some(true),
                capabilities: Some(capabilities()),
                processes: Some(programs()),
                process_env: Some(vec!["NO_COLOR".into()]),
                capability_check: broker.clone(),
                workspace: worktree.map(|worktree| {
                    Arc::new(Worktrees(vec![WorktreeRoot {
                        id: 1,
                        root: worktree.path().to_path_buf(),
                        trusted: true,
                    }])) as Arc<dyn Workspace>
                }),
                ..LoadOptions::default()
            },
            cx,
        )
        .await
        .expect("loads");
        let capabilities = capabilities();
        let capabilities = capabilities.iter().map(String::as_str).collect::<Vec<_>>();
        let mut harness = Harness::load_with_capabilities(&probe_v0_3_wasm(), &capabilities)
            .and_then(|harness| harness.with_processes(programs(), &["NO_COLOR"]))
            .expect("the harness loads the 0.3 probe");
        if broker.is_some() {
            harness.set_broker(TestBroker::granted(Box::new(|_| {
                Err(ui_ext::network::FetchError::Denied)
            })));
        }
        if worktree.is_none() {
            harness.set_worktree(None);
        }
        (host, harness)
    }

    /// Runs probe command `command` in both; the results and effects must match.
    async fn command(host: &UiExtension, harness: &mut Harness, command: &str) -> String {
        let event = UiEvent::InputChanged(1, command.into());
        let reply = host
            .handle_event_v0_2(SURFACE, event.clone())
            .await
            .unwrap_or_else(|error| panic!("{command}: {error:#}"));
        let raw = harness
            .handle_event_raw(SURFACE, event)
            .unwrap_or_else(|error| panic!("{command}: the harness trapped: {error:#}"));
        assert_eq!(debug(&reply.effects), debug(&raw.effects), "{command}");
        let host_value = format!("{:?}", reply.value);
        assert_eq!(
            host_value,
            format!("{:?}", raw.value.expect("a report")),
            "{command}"
        );
        host_value
    }

    #[gpui::test]
    async fn test_spawn_checks_match_the_harness(cx: &mut TestAppContext) {
        let worktree = tempfile::tempdir().expect("a worktree");
        let (host, mut harness) = both(Some(Arc::new(GrantAll)), Some(&worktree), cx).await;
        let mut events = host.take_io_events().expect("once");
        for command_text in [
            format!("spawn -- {MISSING} log"),
            format!("spawn env NODE_OPTIONS=x -- {MISSING} status"),
            format!("spawn env OTHER=1 -- {MISSING} status"),
            "write 99 x".to_string(),
            "kill 99".to_string(),
            "close 99".to_string(),
            "spawn -- undeclared-program".to_string(),
            format!("spawn env NO_COLOR=1 -- {MISSING} status --short"),
            format!("spawn stdin -- {MISSING}-2 a b"),
            format!("spawn -- {MISSING}-2"),
            format!("spawn -- {MISSING}-2"),
        ] {
            command(&host, &mut harness, &command_text).await;
        }
        // Four accepted: the undeclared one and three that cannot start. Their outcomes are
        // `spawn-failed(not-found)` in both, which the probe pushes.
        let mut harness_effects = harness
            .deliver_io()
            .expect("deliver")
            .iter()
            .map(|delivered| debug(&delivered.effects))
            .collect::<Vec<_>>();
        assert_eq!(harness_effects.len(), 4, "{harness_effects:?}");
        let mut host_effects = Vec::new();
        for _ in 0..4 {
            let event = futures::StreamExt::next(&mut events)
                .await
                .expect("an outcome");
            let reply = host.handle_io(event).await.expect("handle-io");
            host_effects.push(debug(&reply.effects));
        }
        harness_effects.sort();
        host_effects.sort();
        assert_eq!(host_effects, harness_effects);
        assert!(
            host_effects
                .iter()
                .flatten()
                .all(|effect| effect.contains("not-found")),
            "{host_effects:?}"
        );
    }

    #[gpui::test]
    async fn test_without_a_broker_or_a_worktree_alike(cx: &mut TestAppContext) {
        let worktree = tempfile::tempdir().expect("a worktree");
        let (host, mut harness) = both(None, Some(&worktree), cx).await;
        let report = command(&host, &mut harness, &format!("spawn -- {MISSING}-2")).await;
        assert!(report.contains("denied"), "{report}");
        // A5: no worktree is its own code.
        let (host, mut harness) = both(Some(Arc::new(GrantAll)), None, cx).await;
        let report = command(&host, &mut harness, &format!("spawn -- {MISSING}-2")).await;
        assert!(report.contains("no-worktree"), "{report}");
    }

    /// The harness's fake programs: stdin, output, the exit code, a kill.
    #[test]
    fn test_fake_programs_run_in_the_harness() {
        let capabilities = capabilities();
        let capabilities = capabilities.iter().map(String::as_str).collect::<Vec<_>>();
        let mut harness = Harness::load_with_capabilities(&probe_v0_3_wasm(), &capabilities)
            .and_then(|harness| harness.with_processes(programs(), &[]))
            .expect("loads");
        harness.set_broker(TestBroker::granted(Box::new(|_| {
            Err(ui_ext::network::FetchError::Denied)
        })));
        harness.fake_program(&format!("{MISSING}-2"), |request, stdin| {
            FakeRun::ok(format!(
                "{}_{}",
                request.args.join(","),
                String::from_utf8_lossy(stdin)
            ))
        });
        let run = |harness: &mut Harness, command: &str| {
            format!(
                "{:?}",
                harness
                    .handle_event_raw(SURFACE, UiEvent::InputChanged(1, command.into()))
                    .expect("no trap")
                    .value
            )
        };
        let lines = |delivered: &[fyx_ext_harness::RawEffects<()>]| {
            delivered
                .iter()
                .map(|delivered| format!("{:?}", delivered.effects))
                .collect::<Vec<_>>()
        };
        let spawned = run(&mut harness, &format!("spawn stdin -- {MISSING}-2 a"));
        assert!(spawned.contains("\"1\""), "{spawned}");
        run(&mut harness, "write 1 hello");
        // Stdin is open: it starts, and waits.
        assert!(harness.deliver_io().expect("deliver").is_empty());
        run(&mut harness, "close 1");
        let delivered = lines(&harness.deliver_io().expect("deliver"));
        assert_eq!(delivered.len(), 2, "{delivered:?}");
        assert!(
            delivered[0].contains("output 1 stdout a_hello"),
            "{delivered:?}"
        );
        assert!(delivered[1].contains("exited 1 0 false"), "{delivered:?}");
        assert!(run(&mut harness, "kill 1").contains("invalid-argument"));

        run(&mut harness, &format!("spawn stdin -- {MISSING}-2"));
        run(&mut harness, "kill 2");
        let delivered = lines(&harness.deliver_io().expect("deliver"));
        assert!(delivered[0].contains("exited 2 none true"), "{delivered:?}");
        assert!(harness.processes().is_empty());
    }
}

/// `workspace:edit` and `agent:observe` (plan 4.8) in the host and the harness: the import's
/// checks, the checks when an edit is applied, the outcomes, and the agent feed with the same
/// opaque ids.
mod harness_conformance_edit_observe {
    use std::sync::Mutex;

    use futures::channel::mpsc;
    use fyx_ext_harness::{Harness, TestBroker};

    use super::*;
    use crate::{
        edit::{EditError, WorkspaceEdit, WorkspaceEdits},
        observe::{AgentEvent, AgentReviews, Resolution, ReviewInfo, ReviewSource},
        permissions::{CapabilityCheck, CapabilityDenied, PermissionState},
    };

    struct GrantAll;

    impl CapabilityCheck for GrantAll {
        fn check(&self, _: &str, _: &str) -> Result<(), CapabilityDenied> {
            Ok(())
        }

        fn state(&self, _: &str, _: &str) -> PermissionState {
            PermissionState::Granted
        }
    }

    /// The editor side: applies every edit it is given.
    struct ApplyAll;

    impl WorkspaceEdits for ApplyAll {
        fn apply(
            &self,
            _: &str,
            _: WorkspaceEdit,
        ) -> futures::future::BoxFuture<'static, Result<(), EditError>> {
            futures::future::ready(Ok(())).boxed()
        }
    }

    /// The agent review side: a fixed snapshot, and a sender for the test to feed.
    #[derive(Default)]
    struct Reviews {
        snapshot: Vec<ReviewInfo>,
        feed: Mutex<Option<mpsc::UnboundedSender<AgentEvent>>>,
    }

    impl AgentReviews for Reviews {
        fn subscribe(
            &self,
        ) -> futures::future::BoxFuture<
            'static,
            (
                Vec<ReviewInfo>,
                futures::stream::BoxStream<'static, AgentEvent>,
            ),
        > {
            let (sender, receiver) = mpsc::unbounded();
            *self.feed.lock().unwrap() = Some(sender);
            futures::future::ready((self.snapshot.clone(), receiver.boxed())).boxed()
        }
    }

    fn review(review: u64) -> ReviewInfo {
        ReviewInfo {
            review,
            session: 70,
            turn: 90,
            file: Some(crate::process::FileRef {
                worktree: Some(1),
                path: "src/main.rs".into(),
            }),
            source: ReviewSource::Write,
            rule: None,
            resolution: Resolution::Pending(0),
            submitted: false,
            hunks: Vec::new(),
            truncated: false,
        }
    }

    fn debug<T: std::fmt::Debug>(values: &[T]) -> Vec<String> {
        values.iter().map(|value| format!("{value:?}")).collect()
    }

    fn capabilities() -> Vec<String> {
        vec![
            CAPABILITY_PANEL.into(),
            ui_ext::v0_2::CAPABILITY_STATUS_BAR.into(),
            ui_ext::v0_2::CAPABILITY_MODAL.into(),
            "workspace:edit".into(),
            "agent:observe".into(),
        ]
    }

    async fn both(
        broker: Option<Arc<dyn CapabilityCheck>>,
        reviews: Arc<Reviews>,
        cx: &mut TestAppContext,
    ) -> (UiExtension, Harness) {
        let host = load_with(
            LoadOptions {
                allow_staged: Some(true),
                capabilities: Some(capabilities()),
                capability_check: broker.clone(),
                workspace_edits: Some(Arc::new(ApplyAll)),
                agent_reviews: Some(reviews.clone() as Arc<dyn AgentReviews>),
                ..LoadOptions::default()
            },
            cx,
        )
        .await
        .expect("loads");
        let capabilities = capabilities();
        let capabilities = capabilities.iter().map(String::as_str).collect::<Vec<_>>();
        let mut harness = Harness::load_with_capabilities(&probe_v0_3_wasm(), &capabilities)
            .expect("the harness loads the 0.3 probe");
        if broker.is_some() {
            harness.set_broker(TestBroker::granted(Box::new(|_| {
                Err(ui_ext::network::FetchError::Denied)
            })));
        }
        harness.set_agent_reviews(Some(reviews.snapshot.clone()));
        (host, harness)
    }

    /// Runs probe command `command` in both; the results and effects must match.
    async fn command(host: &UiExtension, harness: &mut Harness, command: &str) -> String {
        let event = UiEvent::InputChanged(1, command.into());
        let reply = host
            .handle_event_v0_2(SURFACE, event.clone())
            .await
            .unwrap_or_else(|error| panic!("{command}: {error:#}"));
        let raw = harness
            .handle_event_raw(SURFACE, event)
            .unwrap_or_else(|error| panic!("{command}: the harness trapped: {error:#}"));
        assert_eq!(debug(&reply.effects), debug(&raw.effects), "{command}");
        let host_value = format!("{:?}", reply.value);
        assert_eq!(
            host_value,
            format!("{:?}", raw.value.expect("a report")),
            "{command}"
        );
        host_value
    }

    /// The next `count` outcomes in both, as sorted pushes.
    async fn outcomes(
        host: &UiExtension,
        events: &mut mpsc::Receiver<crate::network::IoEvent>,
        harness: &mut Harness,
        count: usize,
    ) -> Vec<Vec<String>> {
        let mut harness_effects = harness
            .deliver_io()
            .expect("deliver")
            .iter()
            .map(|delivered| debug(&delivered.effects))
            .collect::<Vec<_>>();
        let mut host_effects = Vec::new();
        for _ in 0..count {
            let event = futures::StreamExt::next(events).await.expect("an outcome");
            let reply = host.handle_io(event).await.expect("handle-io");
            host_effects.push(debug(&reply.effects));
        }
        harness_effects.sort();
        host_effects.sort();
        assert_eq!(host_effects, harness_effects);
        host_effects
    }

    #[gpui::test]
    async fn test_without_a_broker_both_are_denied(cx: &mut TestAppContext) {
        let (host, mut harness) = both(None, Arc::default(), cx).await;
        let report = command(&host, &mut harness, "edit src/a.rs@0.0-0.0=x").await;
        assert!(report.contains("denied"), "{report}");
        let report = command(&host, &mut harness, "observe start").await;
        assert!(report.contains("denied"), "{report}");
    }

    #[gpui::test]
    async fn test_edits_match_the_harness(cx: &mut TestAppContext) {
        let (host, mut harness) = both(Some(Arc::new(GrantAll)), Arc::default(), cx).await;
        let mut events = host.take_io_events().expect("once");
        for command_text in [
            "edit label= src/a.rs@0.0-0.0=x",
            "edit src/a.rs@0.0-0.0=x",
            "edit .git/config@0.0-0.0=x",
            "edit src/a.rs@0.0-0.2=x src/a.rs@0.1-0.3=y",
        ] {
            command(&host, &mut harness, command_text).await;
        }
        let outcomes = outcomes(&host, &mut events, &mut harness, 3).await;
        let text = format!("{outcomes:?}");
        assert!(text.contains("edit-done 1 ok"), "{text}");
        assert!(text.contains("edit-done 2 err not-allowed"), "{text}");
        assert!(text.contains("edit-done 3 err invalid"), "{text}");
        assert_eq!(harness.applied_edits().len(), 1);
    }

    #[gpui::test]
    async fn test_the_agent_feed_matches_the_harness(cx: &mut TestAppContext) {
        let reviews = Arc::new(Reviews {
            snapshot: vec![review(500), review(501)],
            ..Reviews::default()
        });
        let (host, mut harness) = both(Some(Arc::new(GrantAll)), reviews.clone(), cx).await;
        let mut events = host.take_io_events().expect("once");
        command(&host, &mut harness, "observe start").await;
        let snapshot = outcomes(&host, &mut events, &mut harness, 1).await;
        assert!(
            format!("{snapshot:?}").contains("agent snapshot"),
            "{snapshot:?}"
        );

        // Host ids in, the same opaque ids out.
        let opened = AgentEvent::Opened(review(502));
        reviews
            .feed
            .lock()
            .unwrap()
            .as_ref()
            .expect("subscribed")
            .unbounded_send(opened.clone())
            .unwrap();
        harness.agent_event(opened);
        let opened = outcomes(&host, &mut events, &mut harness, 1).await;
        assert!(format!("{opened:?}").contains("agent opened"), "{opened:?}");

        command(&host, &mut harness, "observe stop").await;
        harness.agent_event(AgentEvent::Closed(502));
        assert!(harness.deliver_io().expect("deliver").is_empty());
        assert!(!harness.is_observing());
    }
}
