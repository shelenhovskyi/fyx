//! Icons of both worlds (design.md, I). A 0.2 component names a frozen [`Icon`], mapped here by an
//! exhaustive `match` with no fallback arm: if upstream removes an `IconName`, this stops
//! compiling instead of an extension's icon changing silently. A 0.1 component names upstream
//! icons by string, resolved as 0.1 always did (`IconName::from_str`, `Blocks` when unknown).

use std::str::FromStr as _;

use ui::IconName;
use ui_ext::v0_2::{Icon, IconRef};

/// How an icon is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolvedIcon {
    Named(IconName),
    /// `spinner`: `LoadCircle`, rotating.
    Spinner,
    /// `blank`: nothing, at icon size.
    Blank,
}

impl ResolvedIcon {
    /// Where only a static icon can go (a dock button): the spinner without motion, `Blocks` for
    /// blank.
    pub fn static_name(self) -> IconName {
        match self {
            ResolvedIcon::Named(name) => name,
            ResolvedIcon::Spinner => IconName::LoadCircle,
            ResolvedIcon::Blank => IconName::Blocks,
        }
    }
}

/// A 0.1 icon name: upstream's snake_case `IconName`, `Blocks` when unknown.
pub fn upstream_icon(name: &str) -> IconName {
    IconName::from_str(name).unwrap_or(IconName::Blocks)
}

pub fn resolve_icon(icon: &IconRef) -> ResolvedIcon {
    match icon {
        IconRef::Frozen(icon) => frozen_icon(*icon),
        IconRef::Upstream(name) => ResolvedIcon::Named(upstream_icon(name)),
    }
}

pub fn frozen_icon(icon: Icon) -> ResolvedIcon {
    use ResolvedIcon::Named;
    match icon {
        Icon::ArrowUp => Named(IconName::ArrowUp),
        Icon::ArrowDown => Named(IconName::ArrowDown),
        Icon::ArrowLeft => Named(IconName::ArrowLeft),
        Icon::ArrowRight => Named(IconName::ArrowRight),
        Icon::ChevronUp => Named(IconName::ChevronUp),
        Icon::ChevronDown => Named(IconName::ChevronDown),
        Icon::ChevronLeft => Named(IconName::ChevronLeft),
        Icon::ChevronRight => Named(IconName::ChevronRight),
        Icon::Expand => Named(IconName::ExpandVertical),
        Icon::Collapse => Named(IconName::ListCollapse),
        Icon::Menu => Named(IconName::Menu),
        Icon::Ellipsis => Named(IconName::Ellipsis),
        Icon::ListTree => Named(IconName::ListTree),
        Icon::ListTodo => Named(IconName::ListTodo),
        Icon::Table => Named(IconName::Table),
        Icon::FileTree => Named(IconName::FileTree),
        Icon::Plus => Named(IconName::Plus),
        Icon::Dash => Named(IconName::Dash),
        Icon::Check => Named(IconName::Check),
        Icon::CheckDouble => Named(IconName::CheckDouble),
        Icon::Close => Named(IconName::Close),
        Icon::Search => Named(IconName::MagnifyingGlass),
        Icon::Filter => Named(IconName::Filter),
        Icon::Refresh => Named(IconName::RotateCw),
        Icon::Undo => Named(IconName::Undo),
        Icon::Copy => Named(IconName::Copy),
        Icon::Pencil => Named(IconName::Pencil),
        Icon::Trash => Named(IconName::Trash),
        Icon::Replace => Named(IconName::Replace),
        Icon::Download => Named(IconName::Download),
        Icon::Send => Named(IconName::Send),
        Icon::Share => Named(IconName::Share),
        Icon::Link => Named(IconName::Link),
        Icon::Pin => Named(IconName::Pin),
        Icon::Unpin => Named(IconName::Unpin),
        Icon::Play => Named(IconName::PlayFilled),
        Icon::Pause => Named(IconName::DebugPause),
        Icon::Stop => Named(IconName::Stop),
        Icon::Maximize => Named(IconName::Maximize),
        Icon::Minimize => Named(IconName::Minimize),
        Icon::Split => Named(IconName::Split),
        Icon::File => Named(IconName::File),
        Icon::FileCode => Named(IconName::FileCode),
        Icon::FileDiff => Named(IconName::FileDiff),
        Icon::FileText => Named(IconName::FileTextOutlined),
        Icon::Folder => Named(IconName::Folder),
        Icon::FolderOpen => Named(IconName::FolderOpen),
        Icon::Code => Named(IconName::Code),
        Icon::Terminal => Named(IconName::Terminal),
        Icon::Diff => Named(IconName::Diff),
        Icon::Regex => Named(IconName::Regex),
        Icon::CaseSensitive => Named(IconName::CaseSensitive),
        Icon::WholeWord => Named(IconName::WholeWord),
        Icon::Hash => Named(IconName::Hash),
        Icon::AtSign => Named(IconName::AtSign),
        Icon::GitBranch => Named(IconName::GitBranch),
        Icon::GitCommit => Named(IconName::GitCommit),
        Icon::PullRequest => Named(IconName::PullRequest),
        Icon::Info => Named(IconName::Info),
        Icon::Warning => Named(IconName::Warning),
        Icon::Error => Named(IconName::XCircle),
        Icon::Help => Named(IconName::CircleHelp),
        Icon::Bell => Named(IconName::Bell),
        Icon::BellDot => Named(IconName::BellDot),
        Icon::Circle => Named(IconName::Circle),
        Icon::Dot => Named(IconName::Indicator),
        Icon::Star => Named(IconName::Star),
        Icon::StarFilled => Named(IconName::StarFilled),
        Icon::Bookmark => Named(IconName::Bookmark),
        Icon::ThumbsUp => Named(IconName::ThumbsUp),
        Icon::ThumbsDown => Named(IconName::ThumbsDown),
        Icon::TodoPending => Named(IconName::TodoPending),
        Icon::TodoProgress => Named(IconName::TodoProgress),
        Icon::TodoComplete => Named(IconName::TodoComplete),
        Icon::Spinner => ResolvedIcon::Spinner,
        Icon::Blank => ResolvedIcon::Blank,
        Icon::Settings => Named(IconName::Settings),
        Icon::Clock => Named(IconName::Clock),
        Icon::Debug => Named(IconName::Debug),
        Icon::Eye => Named(IconName::Eye),
        Icon::EyeOff => Named(IconName::EyeOff),
        Icon::Lock => Named(IconName::Lock),
        Icon::Unlock => Named(IconName::LockOff),
        Icon::Person => Named(IconName::Person),
        Icon::People => Named(IconName::UserGroup),
        Icon::Chat => Named(IconName::Chat),
        Icon::Envelope => Named(IconName::Envelope),
        Icon::Book => Named(IconName::Book),
        Icon::Notepad => Named(IconName::Notepad),
        Icon::Image => Named(IconName::Image),
        Icon::Globe => Named(IconName::Public),
        Icon::Server => Named(IconName::Server),
        Icon::Database => Named(IconName::DatabaseZap),
        Icon::Keyboard => Named(IconName::Keyboard),
        Icon::Box => Named(IconName::Box),
        Icon::Archive => Named(IconName::Archive),
        Icon::Sparkle => Named(IconName::Sparkle),
        Icon::Bolt => Named(IconName::BoltFilled),
        Icon::Flame => Named(IconName::Flame),
        Icon::Power => Named(IconName::Power),
        Icon::Blocks => Named(IconName::Blocks),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_frozen_icon_resolves() {
        assert_eq!(Icon::ALL.len(), 101);
        for icon in Icon::ALL {
            let _ = frozen_icon(*icon);
        }
        assert_eq!(frozen_icon(Icon::Spinner), ResolvedIcon::Spinner);
        assert_eq!(frozen_icon(Icon::Blank), ResolvedIcon::Blank);
        assert_eq!(
            resolve_icon(&IconRef::Upstream("no_such_icon".into())),
            ResolvedIcon::Named(IconName::Blocks)
        );
        assert_eq!(
            resolve_icon(&IconRef::Upstream("hash".into())),
            ResolvedIcon::Named(IconName::Hash)
        );
    }
}
