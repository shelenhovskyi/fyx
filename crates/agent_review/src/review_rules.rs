//! Project review rules: `.fyx/review.toml` at the root of a worktree.
//!
//! ```toml
//! auto_accept = ["docs/**"]                   # land without review (trusted worktrees only)
//! always_ask = ["Cargo.lock", ".github/**"]   # always reviewed, even with review mode off
//! deny = ["*.env"]                            # never written; the agent is told why
//! ```
//!
//! Rules are evaluated per file by the host before a review opens; the first kind that matches wins,
//! in the order `deny`, `always_ask`, `auto_accept`. A pattern without `/` matches the file name at any
//! depth; a pattern with `/` matches the path relative to the worktree root. The file comes with the
//! repository, so it may only make review stricter unless the user trusts the worktree: `deny` and
//! `always_ask` always apply, `auto_accept` only in trusted worktrees.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use globset::{GlobBuilder, GlobMatcher};
use serde::Deserialize;

/// Where the rules live, relative to a worktree root.
pub const RULES_FILE: &str = ".fyx/review.toml";

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
    pub rules_file: PathBuf,
}

impl RuleMatch {
    pub fn describe(&self) -> String {
        format!(
            "{} = \"{}\" in {}",
            self.kind.key(),
            self.pattern,
            self.rules_file.display()
        )
    }
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RulesFile {
    #[serde(default)]
    auto_accept: Vec<String>,
    #[serde(default)]
    always_ask: Vec<String>,
    #[serde(default)]
    deny: Vec<String>,
}

struct Pattern {
    source: String,
    matcher: GlobMatcher,
    file_name_only: bool,
}

impl Pattern {
    fn new(source: &str) -> Result<Self> {
        let file_name_only = !source.contains('/');
        let matcher = GlobBuilder::new(source)
            .literal_separator(true)
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

/// The rules of one worktree.
pub struct ReviewRules {
    root: PathBuf,
    trusted: bool,
    auto_accept: Vec<Pattern>,
    always_ask: Vec<Pattern>,
    deny: Vec<Pattern>,
}

impl ReviewRules {
    pub fn parse(root: PathBuf, text: &str, trusted: bool) -> Result<Self> {
        let file: RulesFile = toml::from_str(text).context("invalid review rules")?;
        let compile = |patterns: &[String]| {
            patterns
                .iter()
                .map(|pattern| Pattern::new(pattern))
                .collect::<Result<Vec<_>>>()
        };
        Ok(Self {
            auto_accept: compile(&file.auto_accept)?,
            always_ask: compile(&file.always_ask)?,
            deny: compile(&file.deny)?,
            root,
            trusted,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn is_trusted(&self) -> bool {
        self.trusted
    }

    /// The rule for `abs_path`, if it lies in this worktree and a rule matches.
    pub fn evaluate(&self, abs_path: &Path) -> Option<RuleMatch> {
        let relative = abs_path.strip_prefix(&self.root).ok()?;
        // Patterns are written with `/`; Windows paths are compared the same way.
        let relative = PathBuf::from(relative.to_string_lossy().replace('\\', "/"));
        let kinds: [(RuleKind, &[Pattern]); 3] = [
            (RuleKind::Deny, &self.deny),
            (RuleKind::AlwaysAsk, &self.always_ask),
            (
                RuleKind::AutoAccept,
                if self.trusted { &self.auto_accept } else { &[] },
            ),
        ];
        kinds.into_iter().find_map(|(kind, patterns)| {
            let pattern = patterns.iter().find(|pattern| pattern.matches(&relative))?;
            Some(RuleMatch {
                kind,
                pattern: pattern.source.clone(),
                rules_file: rules_file_path(&self.root),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RULES: &str = r#"
        auto_accept = ["docs/**", "*.md"]
        always_ask = ["Cargo.lock", ".github/**", "docs/security.md"]
        deny = ["*.env", "secrets/**"]
    "#;

    fn rules(trusted: bool) -> ReviewRules {
        ReviewRules::parse(PathBuf::from("/repo"), RULES, trusted).unwrap()
    }

    fn kind(rules: &ReviewRules, path: &str) -> Option<RuleKind> {
        rules
            .evaluate(&Path::new("/repo").join(path))
            .map(|rule| rule.kind)
    }

    #[test]
    fn each_kind_matches_its_files() {
        let rules = rules(true);
        assert_eq!(
            kind(&rules, "docs/guide/intro.md"),
            Some(RuleKind::AutoAccept)
        );
        assert_eq!(kind(&rules, "README.md"), Some(RuleKind::AutoAccept));
        assert_eq!(kind(&rules, "Cargo.lock"), Some(RuleKind::AlwaysAsk));
        assert_eq!(
            kind(&rules, "crates/x/Cargo.lock"),
            Some(RuleKind::AlwaysAsk)
        );
        assert_eq!(
            kind(&rules, ".github/workflows/ci.yml"),
            Some(RuleKind::AlwaysAsk)
        );
        assert_eq!(kind(&rules, ".env"), Some(RuleKind::Deny));
        assert_eq!(kind(&rules, "config/prod.env"), Some(RuleKind::Deny));
        assert_eq!(kind(&rules, "secrets/key.pem"), Some(RuleKind::Deny));
        assert_eq!(kind(&rules, "src/main.rs"), None);
        assert_eq!(rules.evaluate(Path::new("/elsewhere/.env")), None);
    }

    #[test]
    fn stricter_kinds_win() {
        let rules = rules(true);
        // `docs/security.md` matches auto_accept (`docs/**`) and always_ask.
        assert_eq!(kind(&rules, "docs/security.md"), Some(RuleKind::AlwaysAsk));
        let both = ReviewRules::parse(
            PathBuf::from("/repo"),
            r#"auto_accept = ["**"]
               deny = ["*.env"]"#,
            true,
        )
        .unwrap();
        assert_eq!(kind(&both, "a/.env"), Some(RuleKind::Deny));
        let matched = both.evaluate(Path::new("/repo/a/.env")).unwrap();
        assert_eq!(matched.pattern, "*.env");
        assert!(matched.describe().contains("deny = \"*.env\""));
    }

    #[test]
    fn untrusted_worktrees_cannot_auto_accept() {
        let rules = rules(false);
        assert_eq!(kind(&rules, "docs/guide/intro.md"), None);
        assert_eq!(kind(&rules, "Cargo.lock"), Some(RuleKind::AlwaysAsk));
        assert_eq!(kind(&rules, ".env"), Some(RuleKind::Deny));
    }

    #[test]
    fn bad_files_are_errors() {
        assert!(ReviewRules::parse(PathBuf::from("/r"), "deny = [\"[\"]", true).is_err());
        assert!(ReviewRules::parse(PathBuf::from("/r"), "allow = []", true).is_err());
        assert!(ReviewRules::parse(PathBuf::from("/r"), "", true).is_ok());
    }
}
