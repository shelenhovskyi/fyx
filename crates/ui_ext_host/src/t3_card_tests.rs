//! T3 cards and the "Always allow" warning (0.4, design §C.3).

use std::time::Duration;

use futures::channel::oneshot;
use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use ui_ext::terminal::ReadScope;
use ui_ext_wasm::permissions::{T3Answer, T3Capability, T3Prompt};

use crate::{
    UiExtensionRegistry,
    notifications::CLICK_GUARD,
    t3_cards::{T3Card, always_allow_warning, visible_write_text},
};

fn prompt(capability: T3Capability) -> T3Prompt {
    T3Prompt {
        capability,
        terminal: 7,
        terminal_title: "zsh\u{202e} — demo".into(),
        scope: (capability == T3Capability::Read).then_some((ReadScope::LastLines, 40)),
        text: (capability == T3Capability::Write).then(|| "cargo\ttest\u{200b}".into()),
        submit: true,
    }
}

fn show(
    capability: T3Capability,
    can_always_allow: bool,
    cx: &mut TestAppContext,
) -> (
    Entity<T3Card>,
    oneshot::Receiver<T3Answer>,
    &mut VisualTestContext,
) {
    crate::tests::init_test(cx);
    let registry = cx.new(|_| UiExtensionRegistry::trusting());
    let (sender, receiver) = oneshot::channel();
    let weak = registry.downgrade();
    let (card, vcx) = cx.add_window_view(|_, cx| {
        T3Card::new(
            "fake".into(),
            "Fake".into(),
            prompt(capability),
            sender,
            can_always_allow,
            weak,
            cx,
        )
    });
    vcx.run_until_parked();
    (card, receiver, vcx)
}

#[test]
fn test_t3_write_text_is_shown_exactly() {
    assert_eq!(
        visible_write_text("cargo\ttest\u{200b} \u{202e}x"),
        "cargo⇥test<U+200B> <U+202E>x"
    );
    assert!(always_allow_warning("Fake").starts_with("Fake will see everything"));
}

/// §C.3: a read card offers Once and "until it closes"; the buttons ignore clicks for 1 s, no
/// key answers, and the answer goes back once.
#[gpui::test]
async fn test_t3_read_card(cx: &mut TestAppContext) {
    let (card, mut receiver, vcx) = show(T3Capability::Read, false, cx);
    assert_eq!(
        card.read_with(vcx, |card, _| card.answers()),
        [T3Answer::Deny, T3Answer::UntilClosed, T3Answer::Once]
    );
    vcx.executor()
        .advance_clock(CLICK_GUARD - Duration::from_millis(1));
    card.update(vcx, |card, cx| card.choose(T3Answer::Once, cx));
    vcx.update(|window, cx| gpui::Focusable::focus_handle(card.read(cx), cx).focus(window, cx));
    vcx.simulate_keystrokes("enter space escape");
    vcx.run_until_parked();
    assert_eq!(receiver.try_recv(), Ok(None), "inside the guard, and keys");

    vcx.executor().advance_clock(Duration::from_millis(1));
    // No "Always allow" for a dev folder.
    card.update(vcx, |card, cx| card.ask_always_allow(cx));
    assert!(!card.read_with(vcx, |card, _| card.is_confirming_always()));
    card.update(vcx, |card, cx| card.choose(T3Answer::UntilClosed, cx));
    card.update(vcx, |card, cx| card.choose(T3Answer::Deny, cx));
    assert_eq!(receiver.try_recv(), Ok(Some(T3Answer::UntilClosed)));
}

/// §C.3: a write is confirmed once, never "until it closes"; closing the card is Deny.
#[gpui::test]
async fn test_t3_write_card(cx: &mut TestAppContext) {
    let (card, mut receiver, vcx) = show(T3Capability::Write, false, cx);
    assert_eq!(
        card.read_with(vcx, |card, _| card.answers()),
        [T3Answer::Deny, T3Answer::Once]
    );
    vcx.executor().advance_clock(CLICK_GUARD);
    card.update(vcx, |card, cx| card.choose(T3Answer::UntilClosed, cx));
    assert_eq!(receiver.try_recv(), Ok(None), "not offered");
    card.update(vcx, |card, cx| card.close(cx));
    assert_eq!(receiver.try_recv(), Ok(Some(T3Answer::Deny)));
}

/// §C.3: "Always allow" shows the warning first, its confirm button has its own 1 s guard, and
/// an "Always allow" that cannot be saved (no installed component) answers Deny.
#[gpui::test]
async fn test_t3_always_allow_warning(cx: &mut TestAppContext) {
    let (card, mut receiver, vcx) = show(T3Capability::Read, true, cx);
    vcx.executor().advance_clock(CLICK_GUARD);
    card.update(vcx, |card, cx| card.ask_always_allow(cx));
    assert!(card.read_with(vcx, |card, _| card.is_confirming_always()));
    // The card's own buttons wait while the warning shows.
    card.update(vcx, |card, cx| card.choose(T3Answer::Once, cx));
    card.update(vcx, |card, cx| card.confirm_always_allow(cx));
    assert_eq!(receiver.try_recv(), Ok(None), "the warning's guard");
    card.update(vcx, |card, cx| card.cancel_always_allow(cx));
    card.update(vcx, |card, cx| card.ask_always_allow(cx));
    vcx.executor().advance_clock(CLICK_GUARD);
    card.update(vcx, |card, cx| card.confirm_always_allow(cx));
    assert_eq!(
        receiver.try_recv(),
        Ok(Some(T3Answer::Deny)),
        "\"fake\" is not installed, so nothing was saved"
    );
}

/// The runtime dropped the request (the terminal closed): the card closes.
#[gpui::test]
async fn test_t3_card_closes_when_the_request_goes(cx: &mut TestAppContext) {
    let (card, receiver, vcx) = show(T3Capability::Read, false, cx);
    let dismissed = std::rc::Rc::new(std::cell::Cell::new(false));
    vcx.update(|_, cx| {
        let dismissed = dismissed.clone();
        cx.subscribe(&card, move |_, _: &gpui::DismissEvent, _| {
            dismissed.set(true)
        })
        .detach();
    });
    drop(receiver);
    vcx.run_until_parked();
    assert!(dismissed.get());
}

/// The terminal host's bridge (§C.3): menu shares, what a terminal has shared, and the closed
/// hook, which works from any thread and takes every share of that terminal back.
#[gpui::test]
async fn test_t3_share_bridge(cx: &mut TestAppContext) {
    let registry = cx.new(|_| UiExtensionRegistry::default());
    registry.update(cx, |registry, cx| {
        registry.t3_share_terminal("a", 7, T3Capability::Read, T3Answer::Once, cx);
        registry.t3_share_terminal("a", 7, T3Capability::Write, T3Answer::Once, cx);
        registry.t3_share_terminal("b", 7, T3Capability::Read, T3Answer::UntilClosed, cx);
        registry.t3_share_terminal("b", 8, T3Capability::Read, T3Answer::UntilClosed, cx);
    });
    let shared = |terminal, cx: &mut TestAppContext| {
        let mut shared = registry.read_with(cx, |registry, _| registry.t3_shared(terminal));
        shared.sort_by_key(|(id, capability, _)| (id.clone(), *capability));
        shared
    };
    assert_eq!(
        shared(7, cx),
        [
            ("a".to_string(), T3Capability::Read, T3Answer::Once),
            ("a".to_string(), T3Capability::Write, T3Answer::Once),
            ("b".to_string(), T3Capability::Read, T3Answer::UntilClosed),
        ]
    );
    registry.update(cx, |registry, cx| {
        registry.t3_share_terminal("a", 7, T3Capability::Write, T3Answer::Deny, cx)
    });
    assert_eq!(shared(7, cx).len(), 2);

    let closed = registry.update(cx, |registry, cx| registry.t3_terminal_closed_hook(cx));
    std::thread::spawn(move || closed(7)).join().unwrap();
    cx.run_until_parked();
    assert_eq!(shared(7, cx), []);
    assert_eq!(
        shared(8, cx),
        [("b".to_string(), T3Capability::Read, T3Answer::UntilClosed)]
    );
    // Nothing loaded: nobody to share with.
    assert_eq!(
        registry.read_with(cx, |registry, _| registry.t3_share_targets()),
        []
    );
}
