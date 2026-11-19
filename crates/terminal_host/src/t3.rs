//! T3 on the user's terminals (design `docs/fyx/wit-0.4/design.md` §C.3): the main-thread side of
//! `terminals.request-read` and `request-write` once the runtime's card was answered, the
//! "typed by <extension>" mark on the tab, and the terminal menu's "Share with <extension>…".
//!
//! The consent broker (`ui_ext_host`'s `ConsentStore`) decides who may read or write; this crate
//! does not depend on it. `main.rs` passes a [`ShareBridge`] that maps the menu onto the broker,
//! and a hook for [`crate::set_terminal_closed_hook`] so a closed terminal's shares go.

use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;
use std::time::Duration;

use gpui::{App, Global, SharedString};
use terminal_ext::{MenuEntry, TerminalExt, TerminalId};
use ui_ext::terminal::{ReadScope, TerminalEvent, TerminalSink, check_write_text};

/// How long a tab shows "typed by <extension>" after a T3 write (§C.3).
pub(crate) const TYPED_BY_FOR: Duration = Duration::from_secs(5);

/// Where a T3 answer goes: the request's id and the extension's sink.
pub(crate) struct T3Reply {
    pub id: u64,
    pub sink: TerminalSink,
}

impl fmt::Debug for T3Reply {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("T3Reply").field("id", &self.id).finish()
    }
}

/// What the menu may share, for one extension.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShareTarget {
    pub extension_id: String,
    pub extension_name: String,
    /// The extension declared `terminal:read` / `terminal:write` and is loaded.
    pub can_read: bool,
    pub can_write: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Share {
    /// The next read needs no card.
    ReadOnce,
    /// Every read until the terminal closes needs no card.
    ReadUntilClosed,
    /// Writes are open; each one is still confirmed on its card.
    Write,
}

/// The consent broker, as the terminal menu needs it. Terminals are named by
/// [`TerminalId::as_u64`], the broker's `HostTerminalId`.
pub trait ShareBridge: 'static {
    fn targets(&self, cx: &App) -> Vec<ShareTarget>;
    /// What `terminal` shares now, by extension id.
    fn shared(&self, terminal: u64, cx: &App) -> Vec<(String, Share)>;
    /// Turns one share on or off. Reading once and reading until closed replace each other.
    fn share(&self, extension_id: &str, terminal: u64, share: Share, on: bool, cx: &mut App);
    /// The extensions that hold an "Always allow" for terminals, one entry each with what it
    /// covers, so every user terminal's tab says so.
    fn always_allowed(&self, _cx: &App) -> Vec<AlwaysAllowed> {
        Vec::new()
    }
}

/// One extension's "Always allow" for terminals (§C.3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlwaysAllowed {
    pub extension_name: String,
    /// It may read every terminal without asking.
    pub read: bool,
    /// It may type into shared terminals without asking.
    pub write: bool,
}

impl AlwaysAllowed {
    /// The tab tooltip's line, worded by what the grant covers.
    fn describe(&self) -> String {
        let what = match (self.read, self.write) {
            (true, true) => "may read every terminal and type into shared terminals",
            (true, false) => "may read every terminal",
            (false, true) => "may type into shared terminals",
            (false, false) => "has no terminal access",
        };
        format!("{} {what} (Always allow)", self.extension_name)
    }
}

/// Who has access to a user terminal now, for its tab (§C.3).
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Access {
    /// One line per "Always allow" and share, for the tooltip.
    pub lines: Vec<String>,
    /// The extensions holding an "Always allow", named visibly on the tab.
    pub always: Vec<String>,
}

impl Access {
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// The visible tab label: "<name> has access", or a count for several.
    pub fn label(&self) -> Option<String> {
        match self.always.as_slice() {
            [] => None,
            [one] => Some(format!("{one} has access")),
            [first, rest @ ..] => Some(format!("{first} and {} more have access", rest.len())),
        }
    }
}

struct GlobalShareBridge(Rc<dyn ShareBridge>);

impl Global for GlobalShareBridge {}

/// Set once by `main.rs`; without it the terminal menu has no share entries.
pub fn set_share_bridge(bridge: Rc<dyn ShareBridge>, cx: &mut App) {
    cx.set_global(GlobalShareBridge(bridge));
}

/// Main thread: the display name of the extension whose write a terminal shows, while it shows it.
#[derive(Default)]
struct TypedBy(HashMap<TerminalId, (String, u64)>);

impl Global for TypedBy {}

pub(crate) fn typed_by(id: TerminalId, cx: &App) -> Option<String> {
    cx.try_global::<TypedBy>()
        .and_then(|typed| typed.0.get(&id))
        .map(|(extension, _)| extension.clone())
}

/// The plain text the user agreed to share: the last `lines` non-empty logical lines, no ANSI.
pub(crate) fn read(id: TerminalId, scope: ReadScope, lines: u16, reply: T3Reply, cx: &mut App) {
    let result = match crate::terminal_by_id(id, cx) {
        None => Err("the terminal closed".to_owned()),
        Some(terminal) => {
            let terminal = terminal.read(cx);
            match scope {
                ReadScope::LastLines => Ok(terminal
                    .last_n_non_empty_lines(usize::from(lines))
                    .join("\n")),
                ReadScope::LastCommand => terminal
                    .fyx
                    .last_command_block()
                    .map(|block| block.output)
                    .ok_or_else(|| {
                        "no finished command block: the last command needs shell integration in \
                         this terminal"
                            .to_owned()
                    }),
            }
        }
    };
    (reply.sink)(TerminalEvent::ReadDone(reply.id, result));
}

/// Types the confirmed text (and Enter when `submit`), then marks the tab for [`TYPED_BY_FOR`].
pub(crate) fn write(
    id: TerminalId,
    extension: String,
    text: String,
    submit: bool,
    reply: T3Reply,
    cx: &mut App,
) {
    let result = (|| {
        check_write_text(&text).map_err(|error| error.message)?;
        let terminal = crate::terminal_by_id(id, cx).ok_or("the terminal closed")?;
        terminal.update(cx, |terminal, cx| {
            terminal.input(text.into_bytes());
            if submit {
                terminal.input(b"\r".as_slice());
            }
            cx.emit(terminal::Event::Wakeup);
        });
        Ok::<_, String>(())
    })();
    if result.is_ok() {
        mark_typed_by(id, extension, cx);
    }
    (reply.sink)(TerminalEvent::WriteDone(reply.id, result));
}

/// What "typed by …" names: the extension's display name, as the share menu and the eye do; its
/// id only when no loaded extension has it.
fn display_name(targets: &[ShareTarget], extension_id: String) -> String {
    targets
        .iter()
        .find(|target| target.extension_id == extension_id)
        .map(|target| target.extension_name.clone())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(extension_id)
}

fn mark_typed_by(id: TerminalId, extension: String, cx: &mut App) {
    let targets = cx
        .try_global::<GlobalShareBridge>()
        .map(|bridge| bridge.0.clone())
        .map(|bridge| bridge.targets(cx))
        .unwrap_or_default();
    let name = display_name(&targets, extension);
    let typed = cx.default_global::<TypedBy>();
    let generation = typed.0.get(&id).map_or(0, |(_, generation)| generation + 1);
    typed.0.insert(id, (name, generation));
    cx.spawn(async move |cx| {
        cx.background_executor().timer(TYPED_BY_FOR).await;
        cx.update(|cx| {
            let typed = cx.default_global::<TypedBy>();
            // A later write keeps its own 5 s.
            if typed
                .0
                .get(&id)
                .is_some_and(|(_, current)| *current == generation)
            {
                typed.0.remove(&id);
                if let Some(terminal) = crate::terminal_by_id(id, cx) {
                    terminal.update(cx, |_, cx| cx.emit(terminal::Event::Wakeup));
                }
            }
        });
    })
    .detach();
}

/// The terminal menu's "Share with <extension>…" entries (§C.3): for each loaded extension that
/// declared `terminal:read` or `terminal:write`, what this terminal shares with it now.
pub(crate) fn menu_entries(terminal: &TerminalExt, cx: &App) -> Vec<MenuEntry> {
    let Some(bridge) = cx
        .try_global::<GlobalShareBridge>()
        .map(|bridge| bridge.0.clone())
    else {
        return Vec::new();
    };
    // Never an extension's own terminal: only the user's are shared.
    if terminal.origin().is_some() {
        return Vec::new();
    }
    let id = terminal.id().as_u64();
    let shared = bridge.shared(id, cx);
    let mut entries = Vec::new();
    for target in bridge.targets(cx) {
        if !target.can_read && !target.can_write {
            continue;
        }
        let has = |share: Share| {
            shared
                .iter()
                .any(|(extension, shared)| *extension == target.extension_id && *shared == share)
        };
        entries.push(MenuEntry::Header(
            format!("Share with {}", target.extension_name).into(),
        ));
        let mut toggle = |label: &str, share: Share, checked: bool| {
            let bridge = bridge.clone();
            let extension_id = target.extension_id.clone();
            entries.push(MenuEntry::Toggle {
                label: SharedString::from(label.to_owned()),
                checked,
                on_select: Rc::new(move |_, cx| {
                    bridge.share(&extension_id, id, share, !checked, cx);
                }),
            });
        };
        if target.can_read {
            toggle("Read once", Share::ReadOnce, has(Share::ReadOnce));
            toggle(
                "Read until this terminal closes",
                Share::ReadUntilClosed,
                has(Share::ReadUntilClosed),
            );
        }
        if target.can_write {
            toggle(
                "Allow typing (each command confirmed)",
                Share::Write,
                has(Share::Write),
            );
        }
    }
    if !entries.is_empty() {
        entries.insert(0, MenuEntry::Separator);
    }
    entries
}

/// The consent broker changed: tabs redraw their access marks.
pub fn shares_changed(cx: &mut App) {
    let terminals: Vec<_> = cx
        .try_global::<crate::TerminalsById>()
        .map(|terminals| terminals.0.values().filter_map(|t| t.upgrade()).collect())
        .unwrap_or_default();
    for terminal in terminals {
        terminal.update(cx, |_, cx| cx.emit(terminal::Event::Wakeup));
    }
}

/// §C.3: who may read or type into this user terminal now, for its tab.
pub(crate) fn access(terminal: &TerminalExt, cx: &App) -> Access {
    let Some(bridge) = cx
        .try_global::<GlobalShareBridge>()
        .map(|bridge| bridge.0.clone())
    else {
        return Access::default();
    };
    if terminal.origin().is_some() {
        return Access::default();
    }
    let always = bridge
        .always_allowed(cx)
        .into_iter()
        .filter(|grant| grant.read || grant.write)
        .collect::<Vec<_>>();
    let mut lines: Vec<String> = always.iter().map(AlwaysAllowed::describe).collect();
    let shared = bridge.shared(terminal.id().as_u64(), cx);
    if !shared.is_empty() {
        let targets = bridge.targets(cx);
        let name = |id: &str| {
            targets
                .iter()
                .find(|target| target.extension_id == id)
                .map_or_else(|| id.to_owned(), |target| target.extension_name.clone())
        };
        lines.extend(shared.iter().map(|(id, share)| {
            let what = match share {
                Share::ReadOnce => "may read this terminal once",
                Share::ReadUntilClosed => "may read this terminal until it closes",
                Share::Write => "may type here, each command confirmed",
            };
            format!("{} {what}", name(id))
        }));
    }
    Access {
        lines,
        always: always
            .into_iter()
            .map(|grant| grant.extension_name)
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_allow_wording_follows_the_grant() {
        let grant = |read, write| AlwaysAllowed {
            extension_name: "Probe".into(),
            read,
            write,
        };
        assert_eq!(
            grant(true, false).describe(),
            "Probe may read every terminal (Always allow)"
        );
        assert_eq!(
            grant(false, true).describe(),
            "Probe may type into shared terminals (Always allow)"
        );
        assert_eq!(
            grant(true, true).describe(),
            "Probe may read every terminal and type into shared terminals (Always allow)"
        );
    }

    #[test]
    fn the_tab_names_who_holds_an_always_allow() {
        let access = |always: &[&str]| Access {
            lines: vec!["line".into()],
            always: always.iter().map(|name| name.to_string()).collect(),
        };
        assert_eq!(
            access(&[]).label(),
            None,
            "a share alone keeps to the tooltip"
        );
        assert_eq!(
            access(&["Probe"]).label().as_deref(),
            Some("Probe has access")
        );
        assert_eq!(
            access(&["Probe", "Other", "Third"]).label().as_deref(),
            Some("Probe and 2 more have access")
        );
    }

    #[test]
    fn typed_by_names_the_extension_by_its_display_name() {
        let targets = [ShareTarget {
            extension_id: "fyx-check-probe".into(),
            extension_name: "Check Probe".into(),
            can_read: true,
            can_write: true,
        }];
        assert_eq!(
            display_name(&targets, "fyx-check-probe".into()),
            "Check Probe"
        );
        assert_eq!(
            display_name(&targets, "unloaded".into()),
            "unloaded",
            "an extension that is gone keeps its id"
        );
    }
}
