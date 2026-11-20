//! The privacy report (plan 7.6): for each extension and each agent session, what it read, wrote,
//! fetched, ran and which secrets it used in the last seven days, with a Revoke button per granted
//! capability.
//!
//! Extensions come from their usage journal (design wit-0.4 §C.4; fetches and file edits are
//! summed per host and file by the runtime), agents from the secrets vault's journal (plan 7.4).

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use agent_review::{
    ManageSecrets,
    secrets_vault::{SecretUse, SecretUseOutcome, SecretsVault},
};
use fs::Fs;
use gpui::{
    App, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, SharedString,
    Subscription, Window, actions, prelude::*, rems,
};
use theme::ActiveTheme as _;
use ui::{
    Button, ButtonCommon as _, ButtonSize, ButtonStyle, Clickable as _, Color, Label,
    LabelCommon as _, LabelSize, StyledExt as _, h_flex, v_flex,
};
use ui_ext_wasm::journal::{JournalEntry, JournalKind, JournalPage};
use workspace::{ModalView, Workspace};

use crate::UiExtensionRegistry;

actions!(
    fyx_ui,
    [
        /// Opens the privacy report: what each extension and agent read, wrote, fetched, ran
        /// and which secrets it used this week, with a Revoke button per permission.
        OpenPrivacyReport,
    ]
);

/// How far back the report looks.
pub const REPORT_WINDOW: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// What a line of the report is about, in the order the report lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UseKind {
    Read,
    Wrote,
    Proposed,
    Fetched,
    Ran,
    Listened,
    Secret,
}

impl UseKind {
    fn verb(self) -> &'static str {
        match self {
            UseKind::Read => "Read",
            UseKind::Wrote => "Wrote",
            UseKind::Proposed => "Proposed an edit to",
            UseKind::Fetched => "Fetched from",
            UseKind::Ran => "Ran",
            UseKind::Listened => "Listened on",
            UseKind::Secret => "Used the secret",
        }
    }
}

/// One line: a kind of use of one thing (a host, a file, a program, a secret), summed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UseLine {
    pub kind: UseKind,
    pub what: String,
    pub count: u32,
}

impl UseLine {
    /// How the report words it.
    pub fn text(&self) -> String {
        let line = format!("{} {}", self.kind.verb(), self.what);
        match self.count {
            0 | 1 => line,
            count => format!("{line} ({count} times)"),
        }
    }
}

/// Who the report is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Actor {
    Extension {
        id: Arc<str>,
        name: SharedString,
    },
    /// An agent session by its id; `None` for uses outside a session.
    Agent {
        session: Option<String>,
    },
}

/// One actor's part of the report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActorReport {
    pub actor: Actor,
    pub uses: Vec<UseLine>,
    /// Granted capabilities that Revoke takes back (extensions only).
    pub revocable: Vec<String>,
    /// Some journal entries were lost: the report may be short.
    pub incomplete: bool,
}

fn sum(uses: impl IntoIterator<Item = (UseKind, String, u32)>) -> Vec<UseLine> {
    let mut summed: BTreeMap<(UseKind, String), u32> = BTreeMap::new();
    for (kind, what, count) in uses {
        let total = summed.entry((kind, what)).or_default();
        *total = total.saturating_add(count);
    }
    let mut lines: Vec<UseLine> = summed
        .into_iter()
        .map(|((kind, what), count)| UseLine { kind, what, count })
        .collect();
    // By kind, then the most used first.
    lines.sort_by(|a, b| {
        a.kind
            .cmp(&b.kind)
            .then(b.count.cmp(&a.count))
            .then(a.what.cmp(&b.what))
    });
    lines
}

fn extension_use(entry: &JournalEntry) -> Option<(UseKind, String, u32)> {
    Some(match &entry.kind {
        JournalKind::ProgramStarted { program, .. } => (UseKind::Ran, program.clone(), 1),
        JournalKind::TerminalRead { terminal, .. } => {
            (UseKind::Read, format!("the terminal \"{terminal}\""), 1)
        }
        JournalKind::TerminalWrite { terminal, .. } => {
            (UseKind::Wrote, format!("to the terminal \"{terminal}\""), 1)
        }
        JournalKind::FileEdited { path, count } => (UseKind::Wrote, path.clone(), *count),
        JournalKind::DiscoveryFile {
            path,
            written: true,
        } => (UseKind::Wrote, path.clone(), 1),
        JournalKind::ProposedEdit { path, .. } => (UseKind::Proposed, path.clone(), 1),
        JournalKind::Fetched { host, count } => (UseKind::Fetched, host.clone(), *count),
        JournalKind::ListenerOpened { listener, port } => {
            (UseKind::Listened, format!("port {port} ({listener})"), 1)
        }
        JournalKind::DiscoveryFile { written: false, .. }
        | JournalKind::ProgramExited { .. }
        | JournalKind::Connections { .. }
        | JournalKind::Relay { .. }
        | JournalKind::ConsentChanged { .. } => return None,
    })
}

/// An extension's uses since `since_ms` (milliseconds since the Unix epoch).
pub fn extension_uses(page: &JournalPage, since_ms: u64) -> Vec<UseLine> {
    sum(page
        .entries
        .iter()
        .filter(|entry| entry.at_ms >= since_ms)
        .filter_map(extension_use))
}

/// The agents' secret uses since `since_seconds`, one report per session, sessions in the order
/// they first used a secret.
pub fn agent_reports(journal: &[SecretUse], since_seconds: u64) -> Vec<ActorReport> {
    let mut sessions: Vec<(Option<String>, Vec<(UseKind, String, u32)>)> = Vec::new();
    for secret_use in journal
        .iter()
        .filter(|secret_use| secret_use.unix_seconds >= since_seconds)
    {
        let what = match secret_use.outcome {
            SecretUseOutcome::Allowed => secret_use.secret.clone(),
            SecretUseOutcome::Denied => format!("{} (denied)", secret_use.secret),
            SecretUseOutcome::Missing => format!("{} (missing)", secret_use.secret),
        };
        let line = (UseKind::Secret, what, 1);
        match sessions
            .iter_mut()
            .find(|(session, _)| *session == secret_use.session)
        {
            Some((_, uses)) => uses.push(line),
            None => sessions.push((secret_use.session.clone(), vec![line])),
        }
    }
    sessions
        .into_iter()
        .map(|(session, uses)| ActorReport {
            actor: Actor::Agent { session },
            uses: sum(uses),
            revocable: Vec::new(),
            incomplete: false,
        })
        .collect()
}

/// The whole report as of `now_ms`: extensions (with anything to show) first, then agents.
pub fn build(
    registry: &UiExtensionRegistry,
    secrets: Option<&[SecretUse]>,
    now_ms: u64,
) -> Vec<ActorReport> {
    let window_ms = REPORT_WINDOW.as_millis() as u64;
    let since_ms = now_ms.saturating_sub(window_ms);
    let revocable = registry.revocable();
    let mut reports: Vec<ActorReport> = registry
        .known_extension_ids()
        .into_iter()
        .filter_map(|id| {
            let page = registry.journal(&id);
            let uses = extension_uses(&page, since_ms);
            let granted: Vec<String> = revocable
                .iter()
                .filter(|(extension, _)| **extension == *id)
                .map(|(_, capability)| capability.clone())
                .collect();
            (!uses.is_empty() || !granted.is_empty()).then(|| ActorReport {
                actor: Actor::Extension {
                    name: registry.provenance_name(&id),
                    id,
                },
                uses,
                revocable: granted,
                incomplete: page.incomplete,
            })
        })
        .collect();
    if let Some(secrets) = secrets {
        reports.extend(agent_reports(secrets, since_ms / 1000));
    }
    reports
}

// The dialog

pub(crate) fn register(workspace: &mut Workspace) {
    workspace.register_action(|workspace, _: &OpenPrivacyReport, window, cx| {
        let Some(registry) = UiExtensionRegistry::global(cx) else {
            return;
        };
        let fs = workspace.app_state().fs.clone();
        workspace.toggle_modal(window, cx, move |_, cx| {
            PrivacyReport::new(registry, fs, cx)
        });
    });
}

pub struct PrivacyReport {
    registry: Entity<UiExtensionRegistry>,
    fs: Arc<dyn Fs>,
    focus_handle: FocusHandle,
    /// The last revoke that failed, shown under the title.
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl PrivacyReport {
    fn new(registry: Entity<UiExtensionRegistry>, fs: Arc<dyn Fs>, cx: &mut Context<Self>) -> Self {
        let mut subscriptions = vec![cx.observe(&registry, |_, _, cx| cx.notify())];
        if let Some(vault) = SecretsVault::global(cx) {
            subscriptions.push(cx.observe(&vault, |_, _, cx| cx.notify()));
        }
        // Fresh journals: the page's last reads may be old.
        registry.update(cx, |registry, cx| {
            for id in registry.known_extension_ids() {
                registry.request_journal(&id, true, cx);
            }
        });
        Self {
            registry,
            fs,
            focus_handle: cx.focus_handle(),
            error: None,
            _subscriptions: subscriptions,
        }
    }

    fn revoke(&mut self, extension_id: &str, capability: &str, cx: &mut Context<Self>) {
        let fs = self.fs.clone();
        let result = self.registry.update(cx, |registry, cx| {
            registry.revoke(extension_id, capability, fs, cx)
        });
        self.error = result
            .err()
            .map(|error| format!("Could not revoke {capability}: {error:#}"));
        cx.notify();
    }

    fn cancel(&mut self, _: &menu::Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    fn render_actor(
        &self,
        index: usize,
        report: ActorReport,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let (title, extension_id) = match &report.actor {
            Actor::Extension { id, name } => (name.to_string(), Some(id.clone())),
            Actor::Agent {
                session: Some(session),
            } => (format!("Agent session {session}"), None),
            Actor::Agent { session: None } => ("Agents outside a session".to_string(), None),
        };
        let is_agent = extension_id.is_none();
        v_flex()
            .gap_0p5()
            .child(Label::new(title).size(LabelSize::Small))
            .when(report.uses.is_empty(), |this| {
                this.child(
                    Label::new("Nothing used this week")
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                )
            })
            .children(report.uses.iter().map(|line| {
                Label::new(line.text())
                    .size(LabelSize::XSmall)
                    .color(Color::Muted)
            }))
            .when(report.incomplete, |this| {
                this.child(
                    Label::new("Journal incomplete")
                        .size(LabelSize::XSmall)
                        .color(Color::Warning),
                )
            })
            .when_some(extension_id, |this, id| {
                this.when(!report.revocable.is_empty(), |this| {
                    this.child(
                        h_flex().flex_wrap().gap_1().children(
                            report
                                .revocable
                                .iter()
                                .enumerate()
                                .map(|(button, capability)| {
                                    let id = id.clone();
                                    let capability_name = capability.clone();
                                    Button::new(
                                        SharedString::from(format!(
                                            "fyx-privacy-revoke-{index}-{button}"
                                        )),
                                        format!("Revoke {capability}"),
                                    )
                                    .style(ButtonStyle::Outlined)
                                    .size(ButtonSize::Compact)
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            this.revoke(&id, &capability_name, cx)
                                        },
                                    ))
                                }),
                        ),
                    )
                })
            })
            .when(is_agent, |this| {
                this.child(
                    Button::new(
                        SharedString::from(format!("fyx-privacy-secrets-{index}")),
                        "Manage Secrets",
                    )
                    .style(ButtonStyle::Outlined)
                    .size(ButtonSize::Compact)
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(ManageSecrets), cx)),
                )
            })
    }
}

impl EventEmitter<DismissEvent> for PrivacyReport {}

impl Focusable for PrivacyReport {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ModalView for PrivacyReport {}

impl Render for PrivacyReport {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let secrets = SecretsVault::global(cx).map(|vault| vault.read(cx).journal().to_vec());
        let reports = build(
            self.registry.read(cx),
            secrets.as_deref(),
            ui_ext_wasm::journal::now_ms(),
        );
        let empty = reports.is_empty();
        let actors: Vec<_> = reports
            .into_iter()
            .enumerate()
            .map(|(index, report)| self.render_actor(index, report, cx))
            .collect();
        v_flex()
            .track_focus(&self.focus_handle)
            .elevation_3(cx)
            .w(rems(38.))
            .max_h(rems(40.))
            .p_3()
            .gap_2()
            .bg(cx.theme().colors().elevated_surface_background)
            .on_action(cx.listener(Self::cancel))
            .child(Label::new("Privacy Report: the last 7 days"))
            .when_some(self.error.clone(), |this, error| {
                this.child(Label::new(error).size(LabelSize::Small).color(Color::Error))
            })
            .child(
                v_flex()
                    .id("fyx-privacy-report")
                    .overflow_y_scroll()
                    .gap_3()
                    .when(empty, |this| {
                        this.child(
                            Label::new("No extension or agent used anything this week.")
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        )
                    })
                    .children(actors),
            )
    }
}

#[cfg(test)]
#[path = "privacy_report_tests.rs"]
mod tests;
