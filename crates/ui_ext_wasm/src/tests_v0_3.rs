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
        permissions::{CapabilityCheck, CapabilityDenied, ConsentRequest, ConsentState, T2Consent},
    };

    struct Broker {
        allows_calls: Mutex<bool>,
        state: Mutex<ConsentState>,
        answer: bool,
        prompts: AtomicUsize,
    }

    impl Broker {
        fn new(state: ConsentState, answer: bool) -> Arc<Self> {
            Arc::new(Self {
                allows_calls: Mutex::new(true),
                state: Mutex::new(state),
                answer,
                prompts: AtomicUsize::new(0),
            })
        }

        fn set(&self, state: ConsentState) {
            *self.state.lock().unwrap() = state;
        }
    }

    impl CapabilityCheck for Broker {
        fn check(&self, extension_id: &str, capability: &str) -> Result<(), CapabilityDenied> {
            if *self.allows_calls.lock().unwrap() {
                Ok(())
            } else {
                Err(CapabilityDenied {
                    extension_id: extension_id.into(),
                    capability: capability.into(),
                    reason: "refused".into(),
                })
            }
        }
    }

    impl T2Consent for Broker {
        fn state(&self, _: &str, _: &str) -> ConsentState {
            *self.state.lock().unwrap()
        }

        fn request(
            &self,
            _: &str,
            request: ConsentRequest,
        ) -> futures::future::BoxFuture<'static, bool> {
            assert_eq!(request.capability, "network:fetch");
            assert_eq!(request.domains.len(), 1);
            self.prompts.fetch_add(1, Ordering::SeqCst);
            if self.answer {
                self.set(ConsentState::Granted);
            }
            futures::future::ready(self.answer).boxed()
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
                capability_check: broker
                    .clone()
                    .map(|broker| broker as Arc<dyn CapabilityCheck>),
                t2_consent: broker.map(|broker| broker as Arc<dyn T2Consent>),
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
        let broker = Broker::new(ConsentState::Granted, true);
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
        let broker = Broker::new(ConsentState::Granted, true);
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
        *broker.allows_calls.lock().unwrap() = false;
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

        let refused = Broker::new(ConsentState::Ask, false);
        let extension = load_network(server.port, Some(refused.clone()), true, cx).await;
        let mut events = extension.take_io_events().expect("once");
        assert_eq!(command(&extension, &format!("fetch {url}")).await, ["1"]);
        assert_eq!(
            deliver(&extension, &mut events).await,
            "fetch-done 1 err denied"
        );
        assert_eq!(refused.prompts.load(Ordering::SeqCst), 1);
        assert_eq!(server.connections(), 0, "nothing was sent");

        let allowed = Broker::new(ConsentState::Ask, true);
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
        allowed.set(ConsentState::Denied);
        let reply = extension.handle_io(event).await.expect("handle-io");
        assert!(
            format!("{:?}", reply.effects).contains("fetch-done 2 err denied"),
            "{:?}",
            reply.effects
        );
    }

    /// Without a consent broker (W3's plan 4.5 not wired yet), T2 stays denied.
    #[gpui::test]
    async fn test_t2_is_denied_without_a_consent_broker(cx: &mut TestAppContext) {
        let server = server(cx);
        cx.update(gpui_tokio::init);
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
        let mut events = extension.take_io_events().expect("once");
        let url = format!("http://localhost:{}/hello", server.port);
        assert_eq!(command(&extension, &format!("fetch {url}")).await, ["1"]);
        assert_eq!(
            deliver(&extension, &mut events).await,
            "fetch-done 1 err denied"
        );
    }

    #[gpui::test]
    async fn test_cancel_caps_trap_and_reload(cx: &mut TestAppContext) {
        let server = server(cx);
        let broker = Broker::new(ConsentState::Granted, true);
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

    /// An interface this build does not implement traps alike, and disables both.
    #[gpui::test]
    async fn test_unbuilt_interfaces_trap_in_the_harness(cx: &mut TestAppContext) {
        let host = load(cx).await;
        let mut harness = harness();
        let host_error = host
            .handle_event_v0_2(SURFACE, UiEvent::Click(6))
            .await
            .unwrap_err();
        let harness_error = harness
            .handle_event_raw_v0_2(SURFACE, UiEvent::Click(6))
            .unwrap_err();
        let message = "fyx:ui/permissions@0.3.0#query is not implemented by this build of Fyx yet";
        assert!(
            format!("{host_error:?}").contains(message),
            "{host_error:?}"
        );
        assert!(
            format!("{harness_error:?}").contains(message),
            "{harness_error:?}"
        );
        assert!(harness.disabled().is_some());
    }
}
