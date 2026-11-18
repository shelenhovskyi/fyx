//! Review rules.
//!
//! A project's `.fyx/review.toml` can only make review stricter:
//!
//! ```toml
//! always_ask = ["Cargo.lock", ".github/**"]   # always reviewed, even with review mode off
//! deny = ["*.env"]                            # never written; the agent is told why
//! ```
//!
//! Loosening review is the user's call alone: `auto_accept` patterns come from the user's settings
//! (`agent_review.auto_accept`). An agent can write the project file (through the terminal, or a write
//! the user accepted without noticing what it changes), so an `auto_accept` key there is ignored.
//!
//! Rules are evaluated per file by the host before a review opens; the first kind that matches wins,
//! in the order `deny`, `always_ask`, `auto_accept`. A pattern without `/` matches the file name at any
//! depth; a pattern with `/` matches the path relative to the worktree root. Paths are normalised
//! (`.` and `..` resolved) and, on Windows and macOS, compared without regard to case, as those file
//! systems do.

use std::path::{Component, Path, PathBuf};

use anyhow::{Context as _, Result};
use globset::{GlobBuilder, GlobMatcher};
use serde::Deserialize;

/// Where the project rules live, relative to a worktree root.
pub const RULES_FILE: &str = ".fyx/review.toml";

/// Whether paths match without regard to case: the default file systems of Windows and macOS do.
const CASE_INSENSITIVE: bool = cfg!(any(windows, target_os = "macos"));

/// The rules file of the worktree at `root`, with the platform's separators.
pub fn rules_file_path(root: &Path) -> PathBuf {
    RULES_FILE
        .split('/')
        .fold(root.to_path_buf(), |path, component| path.join(component))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleKind {
    AutoAccept,
    AlwaysAsk,
    Deny,
}

impl RuleKind {
    pub fn key(self) -> &'static str {
        match self {
            RuleKind::AutoAccept => "auto_accept",
            RuleKind::AlwaysAsk => "always_ask",
            RuleKind::Deny => "deny",
        }
    }
}

/// The rule that decided a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleMatch {
    pub kind: RuleKind,
    pub pattern: String,
    /// Where the rule is written: a project's rules file, or the user's settings.
    pub source: String,
}

impl RuleMatch {
    pub fn describe(&self) -> String {
        format!(
            "{} = \"{}\" in {}",
            self.kind.key(),
            self.pattern,
            self.source
        )
    }
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RulesFile {
    /// Accepted for compatibility and ignored: only the user's settings may loosen review.
    #[serde(default)]
    auto_accept: Vec<String>,
    #[serde(default)]
    always_ask: Vec<String>,
    #[serde(default)]
    deny: Vec<String>,
}

pub(crate) struct Pattern {
    source: String,
    matcher: GlobMatcher,
    file_name_only: bool,
}

impl Pattern {
    pub(crate) fn new(source: &str) -> Result<Self> {
        let file_name_only = !source.contains('/');
        let matcher = GlobBuilder::new(source)
            .literal_separator(true)
            .case_insensitive(CASE_INSENSITIVE)
            .build()
            .with_context(|| format!("invalid pattern {source:?}"))?
            .compile_matcher();
        Ok(Self {
            source: source.to_string(),
            matcher,
            file_name_only,
        })
    }

    fn matches(&self, relative: &Path) -> bool {
        if self.file_name_only {
            relative
                .file_name()
                .is_some_and(|name| self.matcher.is_match(Path::new(name)))
        } else {
            self.matcher.is_match(relative)
        }
    }
}

/// The user's `auto_accept` patterns, from settings.
#[derive(Default)]
pub(crate) struct UserAutoAccept(Vec<Pattern>);

impl UserAutoAccept {
    /// Compiles the patterns; invalid ones are logged and left out.
    pub(crate) fn new(patterns: &[String]) -> Self {
        Self(
            patterns
                .iter()
                .filter_map(|pattern| {
                    Pattern::new(pattern)
                        .inspect_err(|error| {
                            log::error!("fyx: agent_review.auto_accept: {error:#}")
                        })
                        .ok()
                })
                .collect(),
        )
    }
}

/// The rules of one worktree. A worktree without a rules file has empty rules, so the user's
/// `auto_accept` patterns still have a root to match against.
pub struct ReviewRules {
    root: PathBuf,
    source: String,
    always_ask: Vec<Pattern>,
    deny: Vec<Pattern>,
    ignored_auto_accept: bool,
}

impl ReviewRules {
    pub fn parse(root: PathBuf, text: &str) -> Result<Self> {
        let file: RulesFile = toml::from_str(text).context("invalid review rules")?;
        let compile = |patterns: &[String]| {
            patterns
                .iter()
                .map(|pattern| Pattern::new(pattern))
                .collect::<Result<Vec<_>>>()
        };
        Ok(Self {
            always_ask: compile(&file.always_ask)?,
            deny: compile(&file.deny)?,
            ignored_auto_accept: !file.auto_accept.is_empty(),
            root,
            source: text.to_string(),
        })
    }

    /// The text the rules were parsed from.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Every restriction of both: what holds while the user has not agreed to drop some of them.
    pub fn union(&self, other: &ReviewRules) -> ReviewRules {
        fn merged(first: &[Pattern], second: &[Pattern]) -> Vec<Pattern> {
            let mut sources = Vec::<&str>::new();
            for pattern in first.iter().chain(second) {
                if !sources.contains(&pattern.source.as_str()) {
                    sources.push(&pattern.source);
                }
            }
            sources
                .into_iter()
                .filter_map(|source| Pattern::new(source).ok())
                .collect()
        }
        ReviewRules {
            root: self.root.clone(),
            source: format!(
                "{}\n# and still in force until confirmed:\n{}",
                other.source, self.source
            ),
            always_ask: merged(&self.always_ask, &other.always_ask),
            deny: merged(&self.deny, &other.deny),
            ignored_auto_accept: other.ignored_auto_accept,
        }
    }

    /// The rules of a worktree without a rules file.
    pub fn none(root: PathBuf) -> Self {
        Self {
            root,
            source: String::new(),
            always_ask: Vec::new(),
            deny: Vec::new(),
            ignored_auto_accept: false,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Whether the file had an `auto_accept` key, which a project file may not set.
    pub fn ignored_auto_accept(&self) -> bool {
        self.ignored_auto_accept
    }

    /// Whether `other` would decide every file the same way.
    pub fn same_as(&self, other: &ReviewRules) -> bool {
        self.root == other.root && self.source == other.source
    }

    /// Whether going from these rules to `new` would let some file through with less review: a
    /// `deny` pattern dropped (or moved to `always_ask`), or an `always_ask` pattern dropped. Such a
    /// change needs the user's confirmation; a change that only adds restrictions does not.
    pub fn loosened_by(&self, new: &ReviewRules) -> Vec<String> {
        let has = |patterns: &[Pattern], source: &str| {
            patterns.iter().any(|pattern| pattern.source == source)
        };
        let mut loosened = Vec::new();
        for pattern in &self.deny {
            if !has(&new.deny, &pattern.source) {
                loosened.push(format!("deny = \"{}\"", pattern.source));
            }
        }
        for pattern in &self.always_ask {
            if !has(&new.always_ask, &pattern.source) && !has(&new.deny, &pattern.source) {
                loosened.push(format!("always_ask = \"{}\"", pattern.source));
            }
        }
        loosened
    }

    /// The path of `abs_path` inside this worktree, or `None` when it lies outside.
    pub fn relative_path(&self, abs_path: &Path) -> Option<PathBuf> {
        let path = normalize(abs_path);
        let root = normalize(&self.root);
        let mut path_components = path.components();
        for root_component in root.components() {
            let component = path_components.next()?;
            let same = if CASE_INSENSITIVE {
                component
                    .as_os_str()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&root_component.as_os_str().to_string_lossy())
            } else {
                component == root_component
            };
            if !same {
                return None;
            }
        }
        // Patterns are written with `/`; Windows paths are compared the same way.
        let relative = path_components
            .as_path()
            .to_string_lossy()
            .replace('\\', "/");
        Some(PathBuf::from(relative))
    }

    /// The rule for `abs_path`, if it lies in this worktree and a rule matches: the project's `deny`
    /// and `always_ask`, then the user's `auto_accept`.
    pub(crate) fn evaluate(
        &self,
        abs_path: &Path,
        user_auto_accept: &UserAutoAccept,
    ) -> Option<RuleMatch> {
        let relative = self.relative_path(abs_path)?;
        let project_file = rules_file_path(&self.root).display().to_string();
        let kinds: [(RuleKind, &[Pattern], &str); 3] = [
            (RuleKind::Deny, &self.deny, &project_file),
            (RuleKind::AlwaysAsk, &self.always_ask, &project_file),
            (
                RuleKind::AutoAccept,
                &user_auto_accept.0,
                "your settings (agent_review.auto_accept)",
            ),
        ];
        kinds.into_iter().find_map(|(kind, patterns, source)| {
            let pattern = patterns.iter().find(|pattern| pattern.matches(&relative))?;
            Some(RuleMatch {
                kind,
                pattern: pattern.source.clone(),
                source: source.to_string(),
            })
        })
    }
}

/// The key a worktree's confirmed rules are remembered under: its normalised root, compared the way
/// the platform's file system compares paths.
pub fn memory_key(root: &Path) -> String {
    let key = normalize(root).to_string_lossy().into_owned();
    if CASE_INSENSITIVE {
        key.to_lowercase()
    } else {
        key
    }
}

/// Resolves `.` and `..` without touching the file system.
fn normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            component => normalized.push(component),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    const RULES: &str = r#"
        always_ask = ["Cargo.lock", ".github/**", "docs/security.md"]
        deny = ["*.env", "secrets/**"]
    "#;

    fn rules() -> ReviewRules {
        ReviewRules::parse(PathBuf::from("/repo"), RULES).unwrap()
    }

    fn user(patterns: &[&str]) -> UserAutoAccept {
        UserAutoAccept::new(&patterns.iter().map(|p| p.to_string()).collect::<Vec<_>>())
    }

    fn kind(
        rules: &ReviewRules,
        user_auto_accept: &UserAutoAccept,
        path: &str,
    ) -> Option<RuleKind> {
        rules
            .evaluate(&Path::new("/repo").join(path), user_auto_accept)
            .map(|rule| rule.kind)
    }

    #[test]
    fn each_kind_matches_its_files() {
        let rules = rules();
        let user = user(&["docs/**", "*.md"]);
        assert_eq!(
            kind(&rules, &user, "docs/guide/intro.md"),
            Some(RuleKind::AutoAccept)
        );
        assert_eq!(kind(&rules, &user, "README.md"), Some(RuleKind::AutoAccept));
        assert_eq!(kind(&rules, &user, "Cargo.lock"), Some(RuleKind::AlwaysAsk));
        assert_eq!(
            kind(&rules, &user, "crates/x/Cargo.lock"),
            Some(RuleKind::AlwaysAsk)
        );
        assert_eq!(
            kind(&rules, &user, ".github/workflows/ci.yml"),
            Some(RuleKind::AlwaysAsk)
        );
        assert_eq!(kind(&rules, &user, ".env"), Some(RuleKind::Deny));
        assert_eq!(kind(&rules, &user, "config/prod.env"), Some(RuleKind::Deny));
        assert_eq!(kind(&rules, &user, "secrets/key.pem"), Some(RuleKind::Deny));
        assert_eq!(kind(&rules, &user, "src/main.rs"), None);
        assert_eq!(rules.evaluate(Path::new("/elsewhere/.env"), &user), None);
    }

    #[test]
    fn stricter_kinds_win() {
        let rules = rules();
        // `docs/security.md` matches the user's auto_accept (`docs/**`) and the project's always_ask.
        let user = user(&["docs/**", "**"]);
        assert_eq!(
            kind(&rules, &user, "docs/security.md"),
            Some(RuleKind::AlwaysAsk)
        );
        assert_eq!(kind(&rules, &user, "a/.env"), Some(RuleKind::Deny));
        let matched = rules.evaluate(Path::new("/repo/a/.env"), &user).unwrap();
        assert_eq!(matched.pattern, "*.env");
        assert!(matched.describe().contains("deny = \"*.env\""));
    }

    #[test]
    fn a_project_file_cannot_auto_accept() {
        // The finding's scenario: the agent writes `auto_accept = ["**"]` into the project file.
        let rules = ReviewRules::parse(
            PathBuf::from("/repo"),
            "auto_accept = [\"**\"]\ndeny = [\"*.env\"]\n",
        )
        .unwrap();
        assert!(rules.ignored_auto_accept());
        let nobody = UserAutoAccept::default();
        assert_eq!(kind(&rules, &nobody, "src/main.rs"), None, "still reviewed");
        assert_eq!(kind(&rules, &nobody, ".env"), Some(RuleKind::Deny));
    }

    #[test]
    fn paths_are_normalised_before_matching() {
        let rules = rules();
        let nobody = UserAutoAccept::default();
        assert_eq!(
            rules
                .evaluate(Path::new("/repo/src/../secrets/./key.pem"), &nobody)
                .map(|r| r.kind),
            Some(RuleKind::Deny)
        );
        assert_eq!(
            rules
                .evaluate(Path::new("/repo/../repo/.env"), &nobody)
                .map(|r| r.kind),
            Some(RuleKind::Deny)
        );
        assert_eq!(
            rules.evaluate(Path::new("/repo/../other/.env"), &nobody),
            None,
            "outside the worktree"
        );
        if CASE_INSENSITIVE {
            assert_eq!(
                rules
                    .evaluate(Path::new("/REPO/Config/PROD.ENV"), &nobody)
                    .map(|r| r.kind),
                Some(RuleKind::Deny)
            );
        }
    }

    #[test]
    fn loosening_is_detected() {
        let current = rules();
        let tighter = ReviewRules::parse(
            PathBuf::from("/repo"),
            r#"
                always_ask = [".github/**", "docs/security.md"]
                deny = ["*.env", "secrets/**", "Cargo.lock", "*.pem"]
            "#,
        )
        .unwrap();
        assert!(
            current.loosened_by(&tighter).is_empty(),
            "always_ask moved to deny is stricter"
        );
        let looser = ReviewRules::parse(PathBuf::from("/repo"), r#"deny = ["*.env"]"#).unwrap();
        assert_eq!(
            current.loosened_by(&looser),
            [
                "deny = \"secrets/**\"",
                "always_ask = \"Cargo.lock\"",
                "always_ask = \".github/**\"",
                "always_ask = \"docs/security.md\""
            ]
        );
        assert!(
            !current
                .loosened_by(&ReviewRules::none(PathBuf::from("/repo")))
                .is_empty()
        );
    }

    #[test]
    fn bad_files_are_errors() {
        assert!(ReviewRules::parse(PathBuf::from("/r"), "deny = [\"[\"]").is_err());
        assert!(ReviewRules::parse(PathBuf::from("/r"), "allow = []").is_err());
        assert!(ReviewRules::parse(PathBuf::from("/r"), "").is_ok());
    }
}
