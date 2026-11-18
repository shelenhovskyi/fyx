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
/// extension; it still loads, because nothing calls it at instantiation.
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
