//! The staged 0.3 world against `fixtures/probe_v0_3`: the `images` interface (plan 4.10), image
//! nodes and their checks, and the 0.3 interfaces this build links as traps.

use gpui::TestAppContext;
use ui_ext::{
    image::{ImageFit, ImageFormat, ImageProps, ImageSource},
    v0_2::{Icon, Props, UiEvent, Update},
};

use super::*;

const SURFACE: &str = "probe";

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
async fn test_v0_3_probe_loads_as_staged_v0_3(cx: &mut TestAppContext) {
    let extension = load(cx).await;
    assert_eq!(extension.api_version(), &Version::new(0, 3, 0));
    assert_eq!(extension.ui_api_version(), ApiVersion::V0_3_0);
    assert_eq!(extension.surfaces_v0_2().len(), 3);

    let error = load_with(
        LoadOptions {
            allow_staged: Some(false),
            ..LoadOptions::default()
        },
        cx,
    )
    .await
    .err()
    .expect("staged");
    assert!(
        format!("{error:#}").contains("targets fyx:ui 0.3.0, which is staged"),
        "{error:#}"
    );
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

/// A 0.3 interface this build does not implement traps when called, which disables the
/// extension; it still loads, because nothing calls it at instantiation. WASI imports at a newer
/// patch version than the host's (std's `HashMap` seed in the probe's `surfaces`) still resolve.
#[gpui::test]
async fn test_v0_3_interfaces_not_built_yet_trap(cx: &mut TestAppContext) {
    let extension = load(cx).await;
    let error = extension
        .handle_event_v0_2(SURFACE, UiEvent::Click(6))
        .await
        .unwrap_err();
    assert!(
        error.downcast_ref::<EffectReplyError>().is_none(),
        "{error:#}"
    );
    assert!(
        format!("{error:?}").contains("fyx:ui/permissions@0.3.0#query is not implemented"),
        "{error:?}"
    );
    let error = extension
        .handle_event_v0_2(SURFACE, UiEvent::Click(1))
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("disabled"), "{error:#}");
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
                "T2",
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
            args: Vec::new(),
            path: None,
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

    /// Decision 26: an untrusted worktree is `denied` without a prompt; no worktree is
    /// `unavailable`.
    #[gpui::test]
    async fn test_worktree_rules(cx: &mut TestAppContext) {
        let mut setup = Setup::new(&["hostname"]);
        setup.trusted = false;
        let extension = setup.load(cx).await;
        assert_eq!(command(&extension, "spawn -- hostname").await, "denied");
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
        assert_eq!(command(&none, "spawn -- hostname").await, "unavailable");
    }

    /// The child sees only the filtered environment, and stdin reaches it.
    #[gpui::test]
    async fn test_environment_and_stdin(cx: &mut TestAppContext) {
        #[cfg(windows)]
        let (env_program, env_args, cat) = ("cmd", "/c set", "sort");
        #[cfg(unix)]
        let (env_program, env_args, cat) = ("env", "", "cat");
        let mut setup = Setup::new(&[env_program, cat]);
        setup.env = vec!["FYX_DECLARED".into(), "FYX_GUEST".into()];
        let mut base: Vec<(OsString, OsString)> = std::env::vars_os().collect();
        base.push(("FYX_DECLARED".into(), "from-fyx".into()));
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
            &format!("spawn env FYX_GUEST=from-guest -- {env_program} {env_args}"),
        )
        .await;
        let (stdout, end) = run_to_end(&extension, &mut events, &id).await;
        assert_eq!(end, format!("exited {id} 0 false"));
        assert!(stdout.contains("FYX_DECLARED=from-fyx"), "{stdout}");
        assert!(stdout.contains("FYX_GUEST=from-guest"), "{stdout}");
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
