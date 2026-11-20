//! Plan 7.9: the agent sessions board. ACP threads of a workspace's project beside its terminals in
//! the Agent Sessions panel, one order for both: the ones that need you first, then working, then
//! the most recent.

use std::time::Instant;

use acp_thread::{AcpThread, ThreadStatus};
use gpui::{App, Entity, SharedString};
use terminal_ext::TerminalStatus;
use terminal_view::TerminalView;

use crate::sessions;
use crate::usage_meters::{self, Cost};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ThreadPhase {
    /// A tool call or a question waits for the user.
    NeedsYou,
    Working,
    Idle,
}

impl ThreadPhase {
    pub(crate) fn of(waiting_for_user: bool, status: ThreadStatus) -> Self {
        if waiting_for_user {
            Self::NeedsYou
        } else if status == ThreadStatus::Generating {
            Self::Working
        } else {
            Self::Idle
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::NeedsYou => "Needs you",
            Self::Working => "Working",
            Self::Idle => "Idle",
        }
    }

    /// The same scale as a terminal's [`sessions::sort_key`]: blocked 4, working 2, idle 0.
    fn rank(self) -> u8 {
        match self {
            Self::NeedsYou => 4,
            Self::Working => 2,
            Self::Idle => 0,
        }
    }
}

pub(crate) struct ThreadRow {
    pub thread: Entity<AcpThread>,
    pub title: SharedString,
    pub phase: ThreadPhase,
    /// The project's worktree names.
    pub worktrees: String,
    /// Lines added and removed and files changed, not yet kept or rejected.
    pub diff: (u32, u32, usize),
    pub cost: Option<Cost>,
    pub context: Option<f32>,
    pub changed: Instant,
}

impl ThreadRow {
    pub(crate) fn meta(&self) -> String {
        meta_line(&self.worktrees, self.diff, self.cost.as_ref(), self.context)
    }
}

/// "worktree · +12 −3 in 2 files · $0.42 · 35% context", leaving out what is unknown.
fn meta_line(
    worktrees: &str,
    (added, removed, files): (u32, u32, usize),
    cost: Option<&Cost>,
    context: Option<f32>,
) -> String {
    let mut parts = Vec::new();
    if !worktrees.is_empty() {
        parts.push(worktrees.to_string());
    }
    if files > 0 {
        let noun = if files == 1 { "file" } else { "files" };
        parts.push(format!("+{added} −{removed} in {files} {noun}"));
    }
    if let Some(cost) = cost {
        parts.push(cost.text());
    }
    if let Some(context) = context {
        parts.push(format!("{:.0}% context", context * 100.0));
    }
    parts.join(" · ")
}

pub(crate) enum BoardRow {
    Terminal(Entity<TerminalView>, TerminalStatus),
    Thread(ThreadRow),
}

impl BoardRow {
    pub(crate) fn needs_you(&self) -> bool {
        match self {
            Self::Terminal(_, status) => status.attention,
            Self::Thread(row) => row.phase == ThreadPhase::NeedsYou,
        }
    }

    fn sort_key(&self) -> (bool, u8, Instant) {
        match self {
            Self::Terminal(_, status) => sessions::sort_key(status),
            Self::Thread(row) => (
                row.phase == ThreadPhase::NeedsYou,
                row.phase.rank(),
                row.changed,
            ),
        }
    }
}

/// The live threads whose project is `project`, as rows.
pub(crate) fn thread_rows(project: &Entity<project::Project>, cx: &App) -> Vec<ThreadRow> {
    let Some(meters) = usage_meters::global(cx) else {
        return Vec::new();
    };
    let meters = meters.read(cx);
    meters
        .threads()
        .into_iter()
        .filter(|(thread, _)| thread.read(cx).project().entity_id() == project.entity_id())
        .map(|(thread, changed)| {
            let read = thread.read(cx);
            let action_log = read.action_log().read(cx);
            let stats = action_log.diff_stats(cx);
            let files = action_log.changed_buffers(cx).count();
            let usage = meters.thread_usage(thread.entity_id());
            let worktrees = read
                .project()
                .read(cx)
                .visible_worktrees(cx)
                .map(|worktree| worktree.read(cx).root_name_str().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            ThreadRow {
                title: read.title().unwrap_or_else(|| "Agent thread".into()),
                phase: ThreadPhase::of(read.is_waiting_for_confirmation(), read.status()),
                worktrees,
                diff: (stats.lines_added, stats.lines_removed, files),
                cost: usage.and_then(|usage| usage.cost.clone()),
                context: usage.and_then(|usage| usage.context_ratio()),
                changed,
                thread,
            }
        })
        .collect()
}

/// Terminals and threads in one list: needs you first, then by state, then the most recent.
pub(crate) fn board(
    terminals: Vec<(Entity<TerminalView>, TerminalStatus)>,
    threads: Vec<ThreadRow>,
) -> Vec<BoardRow> {
    let mut rows: Vec<BoardRow> = terminals
        .into_iter()
        .map(|(view, status)| BoardRow::Terminal(view, status))
        .chain(threads.into_iter().map(BoardRow::Thread))
        .collect();
    rows.sort_by_key(|row| std::cmp::Reverse(row.sort_key()));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_thread_waiting_for_the_user_needs_you_whatever_its_status() {
        assert_eq!(
            ThreadPhase::of(true, ThreadStatus::Generating),
            ThreadPhase::NeedsYou
        );
        assert_eq!(
            ThreadPhase::of(false, ThreadStatus::Generating),
            ThreadPhase::Working
        );
        assert_eq!(
            ThreadPhase::of(false, ThreadStatus::Idle),
            ThreadPhase::Idle
        );
        assert!(ThreadPhase::NeedsYou.rank() > ThreadPhase::Working.rank());
        assert!(ThreadPhase::Working.rank() > ThreadPhase::Idle.rank());
    }

    #[test]
    fn the_meta_line_leaves_out_what_is_unknown() {
        let cost = Cost {
            amount: 0.425,
            currency: "USD".into(),
        };
        assert_eq!(
            meta_line("fyx", (12, 3, 2), Some(&cost), Some(0.35)),
            "fyx · +12 −3 in 2 files · $0.42 · 35% context"
        );
        assert_eq!(meta_line("", (0, 0, 0), None, None), "");
        assert_eq!(meta_line("a, b", (1, 0, 1), None, None), "a, b · +1 −0 in 1 file");
    }
}
