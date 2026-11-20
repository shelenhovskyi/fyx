//! Fyx (design wit-0.4 §F.3, plan 6.7): the marks extensions put on project-panel entries, read
//! from `project`'s [`FileDecorations`] global. Badges are drawn next to the git status letter,
//! with the extensions' tooltips; the colour tints the name only where no git status, ignore or
//! marked colour applies.

use std::sync::Arc;

use git::status::GitSummary;
use gpui::{
    AnyElement, App, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _,
};
use project::{
    ProjectPath,
    fyx_file_decorations::{EntryDecorations, FileDecorationColor, FileDecorations},
};
use ui::{Color, Label, LabelCommon as _, LabelSize, Tooltip, h_flex};
use util::rel_path::RelPath;
use worktree::WorktreeId;

use crate::ProjectPanel;

/// Redraws the panel when an extension's decorations change.
pub(crate) fn follow(cx: &mut Context<ProjectPanel>) {
    let decorations = FileDecorations::global(cx);
    cx.observe(&decorations, |_, _, cx| cx.notify()).detach();
}

/// The entry's decorations; empty when no extension decorates anything.
pub(crate) fn for_entry(
    worktree_id: WorktreeId,
    path: &Arc<RelPath>,
    cx: &App,
) -> EntryDecorations {
    let Some(decorations) = FileDecorations::try_global(cx) else {
        return EntryDecorations::default();
    };
    let decorations = decorations.read(cx);
    if decorations.is_empty() {
        return EntryDecorations::default();
    }
    decorations.for_path(&ProjectPath {
        worktree_id,
        path: path.clone(),
    })
}

/// `color` is what git status, ignoring and marking gave the name; an extension's colour
/// replaces only the plain one.
pub(crate) fn label_color(
    color: Color,
    git_status: GitSummary,
    is_ignored: bool,
    is_marked: bool,
    decorations: &EntryDecorations,
) -> Color {
    let plain = git_status == GitSummary::UNCHANGED && !is_ignored && !is_marked;
    match decorations.color {
        Some(decoration) if plain => match decoration {
            FileDecorationColor::Accent => Color::Accent,
            FileDecorationColor::Warning => Color::Warning,
            FileDecorationColor::Error => Color::Error,
            FileDecorationColor::Success => Color::Success,
            FileDecorationColor::Muted => Color::Muted,
        },
        _ => color,
    }
}

/// The badges, with the extensions' tooltips; `None` without badges.
pub(crate) fn badges(decorations: &EntryDecorations) -> Option<AnyElement> {
    if decorations.badges.is_empty() {
        return None;
    }
    let tooltip = decorations
        .tooltips
        .iter()
        .map(|(extension, tooltip)| format!("{extension}: {tooltip}"))
        .collect::<Vec<_>>()
        .join("\n");
    let badges = h_flex()
        .gap_0p5()
        .children(decorations.badges.iter().map(|badge| {
            Label::new(badge.clone())
                .size(LabelSize::Small)
                .color(Color::Muted)
        }));
    Some(if tooltip.is_empty() {
        badges.into_any_element()
    } else {
        gpui::div()
            .id("fyx-decorations")
            .tooltip(Tooltip::text(tooltip))
            .child(badges)
            .into_any_element()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_label_color_only_where_git_gives_none() {
        let decorations = EntryDecorations {
            color: Some(FileDecorationColor::Warning),
            ..EntryDecorations::default()
        };
        let plain = GitSummary::UNCHANGED;
        assert_eq!(
            label_color(Color::Default, plain, false, false, &decorations),
            Color::Warning
        );
        assert_eq!(
            label_color(Color::Ignored, plain, true, false, &decorations),
            Color::Ignored,
            "ignored wins"
        );
        assert_eq!(
            label_color(Color::Selected, plain, false, true, &decorations),
            Color::Selected,
            "marked wins"
        );
        assert_eq!(
            label_color(Color::Default, plain, false, false, &EntryDecorations::default()),
            Color::Default
        );
        assert!(badges(&EntryDecorations::default()).is_none());
    }
}
