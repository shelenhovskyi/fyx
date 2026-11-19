//! fyx: T3 (design wit-0.4 §C.3) between the terminal host and the consent broker, which do not
//! depend on each other: the terminal menu's "Share with <extension>" and the tabs' access marks
//! read and change `ui_ext_host`'s registry, and a closed terminal ends its shares and cards.
//! Also hands the registry the terminal host's discovery-file writer (plan 5.12) for listeners.

use std::rc::Rc;

use gpui::{App, Entity};
use terminal_host::{Share, ShareBridge, ShareTarget};
use ui_ext_host::UiExtensionRegistry;
use ui_ext_wasm::permissions::{T3Answer, T3Capability};

struct Bridge(Entity<UiExtensionRegistry>);

fn share_of(capability: T3Capability, answer: T3Answer) -> Option<Share> {
    match (capability, answer) {
        (_, T3Answer::Deny) => None,
        (T3Capability::Read, T3Answer::Once) => Some(Share::ReadOnce),
        (T3Capability::Read, T3Answer::UntilClosed) => Some(Share::ReadUntilClosed),
        (T3Capability::Write, _) => Some(Share::Write),
    }
}

fn broker_of(share: Share, on: bool) -> (T3Capability, T3Answer) {
    let (capability, answer) = match share {
        Share::ReadOnce => (T3Capability::Read, T3Answer::Once),
        Share::ReadUntilClosed => (T3Capability::Read, T3Answer::UntilClosed),
        Share::Write => (T3Capability::Write, T3Answer::Once),
    };
    (capability, if on { answer } else { T3Answer::Deny })
}

impl ShareBridge for Bridge {
    fn targets(&self, cx: &App) -> Vec<ShareTarget> {
        self.0
            .read(cx)
            .t3_share_targets()
            .into_iter()
            .map(|target| ShareTarget {
                extension_id: target.extension_id,
                extension_name: target.extension_name.to_string(),
                can_read: target.can_read,
                can_write: target.can_write,
            })
            .collect()
    }

    fn shared(&self, terminal: u64, cx: &App) -> Vec<(String, Share)> {
        self.0
            .read(cx)
            .t3_shared(terminal)
            .into_iter()
            .filter_map(|(id, capability, answer)| Some((id, share_of(capability, answer)?)))
            .collect()
    }

    fn share(&self, extension_id: &str, terminal: u64, share: Share, on: bool, cx: &mut App) {
        let (capability, answer) = broker_of(share, on);
        self.0.update(cx, |registry, cx| {
            registry.t3_share_terminal(extension_id, terminal, capability, answer, cx)
        });
    }

    fn always_allowed(&self, cx: &App) -> Vec<String> {
        let registry = self.0.read(cx);
        let mut names: Vec<String> = registry
            .always_allowed()
            .into_iter()
            .map(|(id, _)| registry.provenance_name(&id).to_string())
            .collect();
        names.dedup();
        names
    }
}

/// After `ui_ext_host::init`: without the registry there is nothing to share with.
pub fn init(cx: &mut App) {
    let Some(registry) = UiExtensionRegistry::global(cx) else {
        return;
    };
    let closed = registry.update(cx, |registry, cx| registry.t3_terminal_closed_hook(cx));
    terminal_host::set_terminal_closed_hook(closed, cx);
    terminal_host::set_share_bridge(Rc::new(Bridge(registry.clone())), cx);
    if let Some(writer) = terminal_host::discovery_writer(cx) {
        // Read by each load, so this is before any extension's listeners open.
        registry.update(cx, |registry, _| registry.set_listener_observer(writer));
    }
    cx.observe(&registry, |_, cx| terminal_host::shares_changed(cx))
        .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_shares_and_broker_answers_map_both_ways() {
        for share in [Share::ReadOnce, Share::ReadUntilClosed, Share::Write] {
            let (capability, answer) = broker_of(share, true);
            assert_eq!(share_of(capability, answer), Some(share));
            let (capability, answer) = broker_of(share, false);
            assert_eq!(answer, T3Answer::Deny);
            assert_eq!(share_of(capability, answer), None);
        }
    }
}
