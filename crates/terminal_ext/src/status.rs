//! A terminal's status as the tab and the agent-sessions list show it, kept up to date on the IO
//! thread from OSC 7501 records and OSC 9;4 progress.

use std::time::Instant;

use crate::osc::program_status_limits::MAX_RECORDS;
use crate::osc::{
    BlockedKind, OscEvent, ProgramStatus, Progress, Protocol, ShellMark, StatusState,
};

/// What the tab shows for a terminal. `state` is never [`StatusState::Clear`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalStatus {
    pub state: StatusState,
    pub kind: Option<BlockedKind>,
    /// Short text (OSC 7501 `title`).
    pub label: Option<String>,
    /// Longer text (OSC 7501 `msg`).
    pub detail: Option<String>,
    /// 0–100; `None` is indeterminate or no progress.
    pub progress: Option<u8>,
    /// Something happened the user has not looked at yet: the program became blocked, finished or
    /// failed, or sent a notification. Cleared when the user focuses the terminal.
    pub attention: bool,
    /// Where the status came from. Always shown, so nothing can pass itself off as another tool.
    pub source: StatusSource,
    pub updated: Instant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StatusSource {
    /// Reported by the program in the terminal. `app` is the program's own claim (OSC 7501 `app`).
    Program {
        protocol: Protocol,
        app: Option<String>,
    },
    /// Set by a UI extension (`terminals.set-status`, `fyx:ui@0.4`). `id` is the extension id.
    Extension { id: String },
}

impl StatusSource {
    /// "Claude Code via OSC 7501", "via OSC 9;4".
    pub fn describe(&self) -> String {
        match self {
            Self::Extension { id } => format!("from extension {id}"),
            Self::Program { protocol, app } => {
                let via = match protocol {
                    Protocol::Osc7501 => "OSC 7501",
                    Protocol::Osc9 => "OSC 9;4",
                    Protocol::Osc7 => "OSC 7",
                    Protocol::Osc99 => "OSC 99",
                    Protocol::Osc133 => "OSC 133",
                    Protocol::Osc633 => "OSC 633",
                    Protocol::Osc777 => "OSC 777",
                    Protocol::Osc1337 => "OSC 1337",
                };
                match app {
                    Some(app) => format!("{} via {via}", app_display_name(app)),
                    None => format!("via {via}"),
                }
            }
        }
    }

    pub fn app(&self) -> Option<&str> {
        match self {
            Self::Program { app, .. } => app.as_deref(),
            Self::Extension { .. } => None,
        }
    }
}

/// Display names for well-known OSC 7501 `app` ids; other ids are shown as they are.
pub fn app_display_name(app: &str) -> &str {
    match app {
        "claude-code" => "Claude Code",
        "codex" => "Codex",
        "gemini-cli" => "Gemini CLI",
        _ => app,
    }
}

#[derive(Debug)]
struct Record {
    status: ProgramStatus,
    updated: Instant,
}

/// A status an extension sets on a terminal. Checked by the caller (`ui_ext::terminal`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtensionStatus {
    pub state: StatusState,
    pub label: String,
    pub detail: Option<String>,
    pub progress: Option<u8>,
    pub attention: bool,
}

#[derive(Debug)]
struct ExtensionRecord {
    extension: String,
    status: ExtensionStatus,
    updated: Instant,
}

#[derive(Debug, Default)]
pub(crate) struct StatusModel {
    records: Vec<Record>,
    progress: Option<(Progress, Instant)>,
    attention: bool,
    extensions: Vec<ExtensionRecord>,
    /// When the user last looked at the terminal: an extension's `attention` older than that is
    /// shown as seen.
    seen_at: Option<Instant>,
}

fn needs_attention(state: StatusState) -> bool {
    matches!(
        state,
        StatusState::Blocked | StatusState::Done | StatusState::Error
    )
}

fn is_under(id: Option<&str>, parent: Option<&str>) -> bool {
    match (id, parent) {
        (_, None) => true,
        (Some(id), Some(parent)) => {
            id == parent
                || id
                    .strip_prefix(parent)
                    .is_some_and(|rest| rest.starts_with('/'))
        }
        (None, Some(_)) => false,
    }
}

impl StatusModel {
    pub(crate) fn apply(&mut self, event: &OscEvent, now: Instant) {
        match event {
            OscEvent::ProgramStatus(status) => self.apply_program_status(status, now),
            OscEvent::Progress(Progress::Remove) => self.progress = None,
            OscEvent::Progress(progress) => {
                if matches!(progress, Progress::Error(_)) {
                    self.attention = true;
                }
                self.progress = Some((*progress, now));
            }
            // A new prompt means the program that reported working/blocked is gone.
            OscEvent::Shell {
                mark: ShellMark::PromptStart,
                ..
            } => self.drop_live_records(),
            OscEvent::Notify { .. } => self.attention = true,
            _ => {}
        }
    }

    fn apply_program_status(&mut self, status: &ProgramStatus, now: Instant) {
        if status.state == StatusState::Clear {
            self.records
                .retain(|record| !is_under(record.status.id.as_deref(), status.id.as_deref()));
            return;
        }
        let previous = self
            .records
            .iter()
            .position(|record| record.status.id == status.id)
            .map(|index| self.records.remove(index));
        if needs_attention(status.state)
            && previous.is_none_or(|previous| previous.status.state != status.state)
        {
            self.attention = true;
        }
        if self.records.len() >= MAX_RECORDS
            && let Some(oldest) = self
                .records
                .iter()
                .enumerate()
                .min_by_key(|(_, record)| record.updated)
                .map(|(index, _)| index)
        {
            self.records.remove(oldest);
        }
        self.records.push(Record {
            status: status.clone(),
            updated: now,
        });
    }

    /// Working and blocked records end with their process (spec: on exit or a new prompt).
    pub(crate) fn drop_live_records(&mut self) {
        self.records.retain(|record| {
            !matches!(
                record.status.state,
                StatusState::Working | StatusState::Blocked
            )
        });
        self.progress = None;
    }

    pub(crate) fn clear_attention(&mut self) {
        self.attention = false;
        self.seen_at = Some(Instant::now());
    }

    pub(crate) fn set_extension_status(
        &mut self,
        extension: &str,
        status: Option<ExtensionStatus>,
        now: Instant,
    ) {
        self.extensions
            .retain(|record| record.extension != extension);
        if let Some(status) = status {
            self.extensions.push(ExtensionRecord {
                extension: extension.to_owned(),
                status,
                updated: now,
            });
        }
    }

    /// What the tab shows: of the program status and the extension statuses, the one that needs
    /// attention, then the newest.
    pub(crate) fn current(&self) -> Option<TerminalStatus> {
        let extensions = self.extensions.iter().map(|record| TerminalStatus {
            state: record.status.state,
            kind: None,
            label: Some(record.status.label.clone()).filter(|label| !label.is_empty()),
            detail: record.status.detail.clone(),
            progress: record.status.progress,
            attention: record.status.attention
                && self.seen_at.is_none_or(|seen| seen < record.updated),
            source: StatusSource::Extension {
                id: record.extension.clone(),
            },
            updated: record.updated,
        });
        self.program_status()
            .into_iter()
            .chain(extensions)
            .max_by_key(|status| (status.attention, status.updated))
    }

    /// The status to show: the most urgent record (blocked, error, working, done, idle; the most
    /// recent first among equals), else OSC 9;4 progress.
    fn program_status(&self) -> Option<TerminalStatus> {
        let rank = |state: StatusState| match state {
            StatusState::Blocked => 4,
            StatusState::Error => 3,
            StatusState::Working => 2,
            StatusState::Done => 1,
            StatusState::Idle | StatusState::Clear => 0,
        };
        if let Some(record) = self
            .records
            .iter()
            .max_by_key(|record| (rank(record.status.state), record.updated))
        {
            let status = &record.status;
            let progress = status.progress.or(match self.progress {
                Some((Progress::Normal(value), _)) => Some(value),
                _ => None,
            });
            return Some(TerminalStatus {
                state: status.state,
                kind: status.kind,
                label: status.title.clone(),
                detail: status.msg.clone(),
                progress,
                attention: self.attention,
                source: StatusSource::Program {
                    protocol: Protocol::Osc7501,
                    app: status.app.clone(),
                },
                updated: record.updated,
            });
        }
        let (progress, updated) = self.progress?;
        let (state, value) = match progress {
            Progress::Normal(value) => (StatusState::Working, Some(value)),
            Progress::Indeterminate => (StatusState::Working, None),
            Progress::Paused(value) => (StatusState::Blocked, value),
            Progress::Error(value) => (StatusState::Error, value),
            Progress::Remove => return None,
        };
        Some(TerminalStatus {
            state,
            kind: None,
            label: None,
            detail: None,
            progress: value,
            attention: self.attention,
            source: StatusSource::Program {
                protocol: Protocol::Osc9,
                app: None,
            },
            updated,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::osc::parse;

    fn apply(model: &mut StatusModel, payload: &str, now: Instant) {
        model.apply(&parse(payload.as_bytes()).unwrap(), now);
    }

    #[test]
    fn claude_code_lifecycle() {
        let start = Instant::now();
        let mut model = StatusModel::default();
        assert_eq!(model.current(), None);

        apply(&mut model, "7501;state=idle:app=claude-code", start);
        let status = model.current().unwrap();
        assert_eq!((status.state, status.attention), (StatusState::Idle, false));
        assert_eq!(status.source.describe(), "Claude Code via OSC 7501");

        apply(&mut model, "7501;state=working:app=claude-code", start);
        assert_eq!(model.current().unwrap().state, StatusState::Working);
        assert!(!model.current().unwrap().attention);

        apply(
            &mut model,
            "7501;state=blocked:kind=permission:app=claude-code",
            start,
        );
        let status = model.current().unwrap();
        assert_eq!(
            (status.state, status.kind, status.attention),
            (StatusState::Blocked, Some(BlockedKind::Permission), true)
        );
        model.clear_attention();

        apply(&mut model, "7501;state=done:app=claude-code", start);
        assert!(model.current().unwrap().attention);
        model.clear_attention();
        apply(&mut model, "7501;state=done:app=claude-code", start);
        assert!(!model.current().unwrap().attention, "same state again");

        apply(&mut model, "7501;state=clear", start);
        assert_eq!(model.current(), None);
    }

    #[test]
    fn the_most_urgent_record_wins_and_clear_removes_children() {
        let start = Instant::now();
        let mut model = StatusModel::default();
        apply(&mut model, "7501;state=working", start);
        apply(
            &mut model,
            "7501;state=blocked:id=build/test",
            start + Duration::from_secs(1),
        );
        apply(
            &mut model,
            "7501;state=done:id=lint",
            start + Duration::from_secs(2),
        );
        assert_eq!(model.current().unwrap().state, StatusState::Blocked);
        apply(&mut model, "7501;state=clear:id=build", start);
        assert_eq!(model.current().unwrap().state, StatusState::Working);
    }

    #[test]
    fn records_are_capped_and_live_ones_end_with_a_new_prompt() {
        let start = Instant::now();
        let mut model = StatusModel::default();
        for index in 0..MAX_RECORDS + 3 {
            apply(
                &mut model,
                &format!("7501;state=done:id=r{index}"),
                start + Duration::from_millis(index as u64),
            );
        }
        assert_eq!(model.records.len(), MAX_RECORDS);
        assert!(
            !model
                .records
                .iter()
                .any(|r| r.status.id.as_deref() == Some("r0"))
        );

        apply(
            &mut model,
            "7501;state=working:id=w",
            start + Duration::from_secs(9),
        );
        assert_eq!(model.current().unwrap().state, StatusState::Working);
        apply(&mut model, "133;A", start);
        assert_eq!(model.current().unwrap().state, StatusState::Done);
    }

    #[test]
    fn extension_statuses_show_beside_the_programs() {
        let start = Instant::now();
        let mut model = StatusModel::default();
        apply(&mut model, "7501;state=working:app=claude-code", start);
        let status = |state, attention| ExtensionStatus {
            state,
            label: "Review".into(),
            detail: None,
            progress: None,
            attention,
        };
        let later = |seconds| start + Duration::from_secs(seconds);
        model.set_extension_status("acme", Some(status(StatusState::Done, false)), later(1));
        let shown = model.current().unwrap();
        assert_eq!(shown.source.describe(), "from extension acme");
        assert_eq!(shown.label.as_deref(), Some("Review"));

        // A newer program report wins over an older extension status without attention.
        apply(&mut model, "7501;state=working:app=claude-code", later(2));
        assert_eq!(
            model.current().unwrap().source.describe(),
            "Claude Code via OSC 7501"
        );

        // Attention wins; looking at the terminal shows it as seen.
        model.set_extension_status("acme", Some(status(StatusState::Blocked, true)), later(3));
        assert!(model.current().unwrap().attention);
        model.seen_at = Some(later(4));
        assert!(!model.current().unwrap().attention);

        model.set_extension_status("acme", None, start);
        assert_eq!(
            model.current().unwrap().source.describe(),
            "Claude Code via OSC 7501"
        );
    }

    #[test]
    fn conemu_progress_without_a_record() {
        let start = Instant::now();
        let mut model = StatusModel::default();
        apply(&mut model, "9;4;1;30", start);
        let status = model.current().unwrap();
        assert_eq!(
            (status.state, status.progress),
            (StatusState::Working, Some(30))
        );
        assert_eq!(status.source.describe(), "via OSC 9;4");
        apply(&mut model, "9;4;0", start);
        assert_eq!(model.current(), None);
    }
}
