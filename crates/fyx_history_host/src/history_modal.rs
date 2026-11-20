//! The "Local History" modal: agent turns and restores of a worktree with "Undo Turn", the active
//! file's versions with "Restore This Version", and the conflict prompt with "Restore Anyway".

use std::path::PathBuf;

use fyx_history::{ChangeKind, ChangeSummary, Content, Origin, RestoreSource, Snapshot};
use gpui::{DismissEvent, EventEmitter, FocusHandle, Focusable, Task, Window};
use ui::{Button, ButtonStyle, Label, LabelSize, prelude::*};
use workspace::{ModalView, Workspace};

use crate::{RestoreOutcome, Timeline, load_timeline, now_ms, restore, settings};

/// Rows shown per section; older history stays in the store until pruned.
const ROWS_SHOWN: usize = 50;

pub(crate) fn open(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let project = workspace.project().read(cx);
    let active = workspace
        .active_item(cx)
        .and_then(|item| item.project_path(cx))
        .and_then(|project_path| {
            let worktree = project.worktree_for_id(project_path.worktree_id, cx)?;
            let root = worktree.read(cx).abs_path().to_path_buf();
            Some((root, Some(PathBuf::from(project_path.path.as_unix_str()))))
        });
    let target = active.or_else(|| {
        project
            .visible_worktrees(cx)
            .next()
            .map(|worktree| (worktree.read(cx).abs_path().to_path_buf(), None))
    });
    workspace.toggle_modal(window, cx, |_, cx| LocalHistoryModal::new(target, cx));
}

pub(crate) struct LocalHistoryModal {
    /// The worktree and the active file in it, if any.
    target: Option<(PathBuf, Option<PathBuf>)>,
    timeline: Option<Timeline>,
    /// A restore refused because these files changed since.
    conflict: Option<(RestoreSource, Vec<String>)>,
    message: Option<(SharedString, bool)>,
    pending: Task<()>,
    focus_handle: FocusHandle,
}

impl LocalHistoryModal {
    fn new(target: Option<(PathBuf, Option<PathBuf>)>, cx: &mut Context<Self>) -> Self {
        let mut modal = Self {
            target,
            timeline: None,
            conflict: None,
            message: None,
            pending: Task::ready(()),
            focus_handle: cx.focus_handle(),
        };
        modal.refresh(cx);
        modal
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some((worktree, file)) = self.target.clone() else {
            return;
        };
        let load = load_timeline(worktree, file, cx);
        self.pending = cx.spawn(async move |this, cx| {
            let result = load.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(timeline) => this.timeline = Some(timeline),
                    Err(error) => this.message = Some((format!("{error:#}").into(), true)),
                }
                cx.notify();
            })
            .ok();
        });
    }

    fn restore(&mut self, source: RestoreSource, overwrite: bool, cx: &mut Context<Self>) {
        let Some((worktree, _)) = self.target.clone() else {
            return;
        };
        self.conflict = None;
        let task = restore(worktree, source, overwrite, cx);
        self.pending = cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(RestoreOutcome::Restored) => {
                        this.message = Some((
                            "Restored. The restore is at the top of the list and can be undone."
                                .into(),
                            false,
                        ));
                    }
                    Ok(RestoreOutcome::Conflicts(paths)) => {
                        this.message = None;
                        this.conflict = Some((source, paths));
                    }
                    Err(error) => this.message = Some((format!("{error:#}").into(), true)),
                }
                this.refresh(cx);
            })
            .ok();
        });
        cx.notify();
    }

    fn render_change(
        &self,
        index: usize,
        summary: &ChangeSummary,
        now: u64,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (title, button) = match &summary.change.kind {
            ChangeKind::AgentTurn { label, .. } => (
                label.clone().unwrap_or_else(|| "Agent turn".to_string()),
                "Undo Turn",
            ),
            ChangeKind::Restore {
                of: RestoreSource::Change(_),
            } => ("Undo".to_string(), "Undo Restore"),
            ChangeKind::Restore {
                of: RestoreSource::Snapshot(_),
            } => ("Restored a version".to_string(), "Undo Restore"),
        };
        let source = RestoreSource::Change(summary.change.id);
        let blocked = summary
            .files
            .iter()
            .any(|file| matches!(file.before, Content::TooLarge { .. }));
        h_flex()
            .justify_between()
            .gap_2()
            .child(
                v_flex()
                    .child(Label::new(format!(
                        "{title} · {}",
                        age_label(now, summary.change.time_ms)
                    )))
                    .child(
                        Label::new(files_label(
                            summary.files.iter().map(|file| file.path.as_str()),
                        ))
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                    ),
            )
            .child(
                Button::new(("undo-change", index), button)
                    .disabled(blocked || summary.files.is_empty())
                    .on_click(cx.listener(move |this, _, _, cx| this.restore(source, false, cx))),
            )
    }

    fn render_snapshot(
        &self,
        index: usize,
        snapshot: &Snapshot,
        now: u64,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let kind = match snapshot.origin {
            Origin::Save => "Saved",
            Origin::Change { change } => self
                .timeline
                .as_ref()
                .and_then(|timeline| {
                    timeline
                        .changes
                        .iter()
                        .find(|summary| summary.change.id == change)
                })
                .map_or("Changed", |summary| match summary.change.kind {
                    ChangeKind::AgentTurn { .. } => "Agent turn",
                    ChangeKind::Restore { .. } => "Restored",
                }),
        };
        let detail = match &snapshot.content {
            Content::Missing => "deleted".to_string(),
            Content::Stored { len, .. } => format!("{len} bytes"),
            Content::TooLarge { len, .. } => format!("{len} bytes, too large to keep"),
        };
        let source = RestoreSource::Snapshot(snapshot.id);
        h_flex()
            .justify_between()
            .gap_2()
            .child(
                v_flex()
                    .child(Label::new(format!(
                        "{kind} · {}",
                        age_label(now, snapshot.time_ms)
                    )))
                    .child(
                        Label::new(detail)
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
            )
            .child(
                Button::new(("restore-snapshot", index), "Restore This Version")
                    .disabled(matches!(snapshot.content, Content::TooLarge { .. }))
                    .on_click(cx.listener(move |this, _, _, cx| this.restore(source, false, cx))),
            )
    }
}

impl EventEmitter<DismissEvent> for LocalHistoryModal {}

impl ModalView for LocalHistoryModal {}

impl Focusable for LocalHistoryModal {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for LocalHistoryModal {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = now_ms();
        let enabled = settings(cx).enabled;
        let file_name = self
            .target
            .as_ref()
            .and_then(|(_, file)| file.as_ref())
            .map(|file| file.to_string_lossy().replace('\\', "/"));
        let (changes, snapshots) = match &self.timeline {
            Some(timeline) => (
                timeline
                    .changes
                    .iter()
                    .take(ROWS_SHOWN)
                    .enumerate()
                    .map(|(index, summary)| {
                        self.render_change(index, summary, now, cx)
                            .into_any_element()
                    })
                    .collect::<Vec<_>>(),
                timeline
                    .file
                    .iter()
                    .take(ROWS_SHOWN)
                    .enumerate()
                    .map(|(index, snapshot)| {
                        self.render_snapshot(index, snapshot, now, cx)
                            .into_any_element()
                    })
                    .collect::<Vec<_>>(),
            ),
            None => (Vec::new(), Vec::new()),
        };
        let section = |title: String| Label::new(title).size(LabelSize::Small).color(Color::Muted);
        v_flex()
            .key_context("LocalHistory")
            .track_focus(&self.focus_handle)
            .w(rems(36.))
            .elevation_3(cx)
            .p_3()
            .gap_2()
            .child(Label::new("Local History"))
            .when(!enabled, |this| {
                this.child(
                    Label::new(
                        "Recording is off (fyx.history.enabled). What is already kept can still be restored.",
                    )
                    .size(LabelSize::Small)
                    .color(Color::Warning),
                )
            })
            .when(self.target.is_none(), |this| {
                this.child(
                    Label::new("Open a folder to see its history.")
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                )
            })
            .when_some(self.conflict.clone(), |this, (source, paths)| {
                this.child(
                    v_flex()
                        .gap_1()
                        .p_2()
                        .border_1()
                        .rounded_sm()
                        .border_color(cx.theme().colors().border)
                        .child(
                            Label::new(format!(
                                "Changed since: {}. Restoring overwrites these edits; the restore can be undone.",
                                paths.join(", ")
                            ))
                            .size(LabelSize::Small)
                            .color(Color::Warning),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .justify_end()
                                .child(Button::new("cancel-restore", "Cancel").on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.conflict = None;
                                        cx.notify();
                                    }),
                                ))
                                .child(
                                    Button::new("restore-anyway", "Restore Anyway")
                                        .style(ButtonStyle::Filled)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.restore(source, true, cx)
                                        })),
                                ),
                        ),
                )
            })
            .when_some(self.message.clone(), |this, (message, error)| {
                this.child(Label::new(message).size(LabelSize::Small).color(if error {
                    Color::Error
                } else {
                    Color::Success
                }))
            })
            .child(
                v_flex()
                    .id("local-history-rows")
                    .max_h(rems(32.))
                    .overflow_y_scroll()
                    .gap_2()
                    .child(section("Agent turns and restores".to_string()))
                    .when(changes.is_empty(), |this| {
                        this.child(
                            Label::new("None yet.")
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        )
                    })
                    .children(changes)
                    .when_some(file_name, |this, file_name| {
                        this.child(section(format!("Versions of {file_name}")))
                            .when(snapshots.is_empty(), |this| {
                                this.child(
                                    Label::new("None yet: a version is kept on every save.")
                                        .size(LabelSize::Small)
                                        .color(Color::Muted),
                                )
                            })
                            .children(snapshots)
                    }),
            )
    }
}

fn age_label(now_ms: u64, time_ms: u64) -> String {
    let minutes = now_ms.saturating_sub(time_ms) / 60_000;
    match minutes {
        0 => "just now".to_string(),
        1..=59 => format!("{minutes} min ago"),
        60..=1439 => format!("{} h ago", minutes / 60),
        _ => format!("{} days ago", minutes / 1440),
    }
}

fn files_label<'a>(paths: impl Iterator<Item = &'a str>) -> String {
    let paths: Vec<&str> = paths.collect();
    match paths.len() {
        0 => "no files".to_string(),
        1..=3 => paths.join(", "),
        count => format!("{} and {} more", paths[..3].join(", "), count - 3),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages_and_file_lists_read_short() {
        let minute = 60_000;
        assert_eq!(age_label(10 * minute, 10 * minute), "just now");
        assert_eq!(age_label(50 * minute, 5 * minute), "45 min ago");
        assert_eq!(age_label(200 * minute, 0), "3 h ago");
        assert_eq!(age_label(3000 * minute, 0), "2 days ago");
        assert_eq!(files_label(["a.rs"].into_iter()), "a.rs");
        assert_eq!(
            files_label(["a", "b", "c", "d", "e"].into_iter()),
            "a, b, c and 2 more"
        );
    }
}
