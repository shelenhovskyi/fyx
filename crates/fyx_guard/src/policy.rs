//! What the shield protects, independent of GPUI: agent config paths, a repository's
//! `.fyx/security.toml`, and the secret-looking environment variables.

use std::path::{Component, Path};

use anyhow::{Context as _, Result};
use serde::Deserialize;
use util::{
    paths::{PathMatcher, PathStyle},
    rel_path::RelPath,
};

/// A repository's own tightening, relative to a worktree root.
pub const SECURITY_FILE: &str = ".fyx/security.toml";

/// Directories agents may not write into, at any depth.
const PROTECTED_DIRS: &[&str] = &[".claude", ".codex", ".zed", ".fyx"];
/// Files agents may not write, at any depth.
const PROTECTED_FILES: &[&str] = &[".mcp.json"];

/// Variables that look like secrets. `*` matches any run of characters; names are compared in
/// ASCII upper case.
pub const SECRET_ENV_PATTERNS: &[&str] = &[
    "AWS_*",
    "GITHUB_TOKEN",
    "GH_TOKEN",
    "NPM_TOKEN",
    "*_API_KEY",
    "*_SECRET*",
    "*_TOKEN",
    "SSH_AUTH_SOCK",
];

/// Names the agent-config rule when `path` is inside one of the agent config locations.
pub fn protected_config(path: &Path) -> Option<String> {
    let names = util::paths::normalize_path(path)
        .components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>();
    protected_config_in(&names)
}

/// The same rule for a path relative to a worktree root.
pub fn protected_config_rel(path: &RelPath) -> Option<String> {
    let names = path.components().map(str::to_string).collect::<Vec<_>>();
    protected_config_in(&names)
}

fn protected_config_in(names: &[String]) -> Option<String> {
    // Case-insensitive file systems (macOS, Windows) resolve `.Claude` to `.claude`.
    let is = |name: &str, protected: &str| name.eq_ignore_ascii_case(protected);
    for (index, name) in names.iter().enumerate() {
        let is_last = index + 1 == names.len();
        if let Some(dir) = PROTECTED_DIRS.iter().find(|dir| is(name, dir)) {
            return Some(format!("{dir}/"));
        }
        if is_last && let Some(file) = PROTECTED_FILES.iter().find(|file| is(name, file)) {
            return Some((*file).to_string());
        }
        if is(name, ".git")
            && let Some(next) = names.get(index + 1)
            && is(next, "hooks")
        {
            return Some(".git/hooks/".to_string());
        }
    }
    None
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct SecurityFile {
    #[serde(default)]
    private_files: Vec<String>,
    #[serde(default)]
    read_only: Vec<String>,
}

/// A parsed `.fyx/security.toml`. It can only add patterns: there is no key that removes a
/// protection, sets the mode or allows a variable, and any unknown key makes the file invalid.
pub struct RepoPolicy {
    private_files: PathMatcher,
    read_only: PathMatcher,
}

impl RepoPolicy {
    pub fn parse(text: &str) -> Result<Self> {
        let file: SecurityFile = toml::from_str(text).context("invalid TOML")?;
        Ok(Self {
            private_files: PathMatcher::new(&file.private_files, PathStyle::Unix)
                .context("invalid pattern in `private_files`")?,
            read_only: PathMatcher::new(&file.read_only, PathStyle::Unix)
                .context("invalid pattern in `read_only`")?,
        })
    }

    /// The `private_files` pattern that `path` (or one of its ancestors) matches.
    pub fn private_match(&self, path: &RelPath) -> Option<String> {
        first_match(&self.private_files, path)
    }

    /// The `read_only` pattern that `path` (or one of its ancestors) matches.
    pub fn read_only_match(&self, path: &RelPath) -> Option<String> {
        first_match(&self.read_only, path)
    }
}

// Same ancestor walk as `WorktreeSettings::is_path_private`, so `secrets/**` and `secrets` both
// cover `secrets/a/b`.
fn first_match(matcher: &PathMatcher, path: &RelPath) -> Option<String> {
    if !path.ancestors().any(|ancestor| matcher.is_match(ancestor)) {
        return None;
    }
    let patterns = matcher.sources().collect::<Vec<_>>();
    Some(patterns.join(", "))
}

/// Where a worktree's repository policy stands.
pub enum RepoState {
    Loading,
    Loaded(Option<RepoPolicy>),
    Failed(String),
}

impl RepoState {
    /// Why an agent may not read `path`, if the repository makes it private or its policy is
    /// unusable (fail closed).
    pub fn read_denial(&self, path: &RelPath) -> Option<String> {
        match self {
            RepoState::Loaded(None) => None,
            RepoState::Loaded(Some(policy)) => policy.private_match(path).map(|patterns| {
                format!("it matches `private_files = [{patterns}]` in {SECURITY_FILE}")
            }),
            RepoState::Loading => {
                Some(format!("{SECURITY_FILE} of this worktree is still loading"))
            }
            RepoState::Failed(error) => Some(format!(
                "{SECURITY_FILE} of this worktree is unusable: {error}"
            )),
        }
    }

    /// Why an agent may not write `path`, under the same rules.
    pub fn write_denial(&self, path: &RelPath) -> Option<String> {
        match self {
            RepoState::Loaded(None) => None,
            RepoState::Loaded(Some(policy)) => policy.read_only_match(path).map(|patterns| {
                format!("it matches `read_only = [{patterns}]` in {SECURITY_FILE}")
            }),
            RepoState::Loading | RepoState::Failed(_) => self.read_denial(path),
        }
    }
}

/// Whether `name` matches `pattern`, where `*` matches any run of characters. ASCII case is
/// ignored: Windows treats variable names case-insensitively, and a lower-case secret is still one.
pub fn env_name_matches(pattern: &str, name: &str) -> bool {
    let pattern = pattern.to_ascii_uppercase();
    let name = name.to_ascii_uppercase();
    let mut parts = pattern.split('*');
    let Some(first) = parts.next() else {
        return name.is_empty();
    };
    let Some(mut rest) = name.strip_prefix(first) else {
        return false;
    };
    let parts = parts.collect::<Vec<_>>();
    let Some((last, middle)) = parts.split_last() else {
        return rest.is_empty();
    };
    for part in middle {
        match rest.find(part) {
            Some(position) => rest = &rest[position + part.len()..],
            None => return false,
        }
    }
    rest.len() >= last.len() && rest.ends_with(last)
}

/// The variables of `names` an agent terminal must not inherit: secret-looking and not allowed.
pub fn secret_env_names<'a>(
    names: impl IntoIterator<Item = &'a String>,
    allow: &[String],
) -> Vec<String> {
    let mut secret = names
        .into_iter()
        .filter(|name| {
            SECRET_ENV_PATTERNS
                .iter()
                .any(|pattern| env_name_matches(pattern, name))
                && !allow.iter().any(|pattern| env_name_matches(pattern, name))
        })
        .cloned()
        .collect::<Vec<_>>();
    secret.sort();
    secret
}

#[cfg(test)]
mod tests {
    use super::*;
    use util::rel_path::rel_path;

    #[test]
    fn agent_configs_are_protected_at_any_depth_and_case() {
        for path in [
            "/p/.claude/settings.json",
            "/p/sub/.claude/commands/x.md",
            "/p/.CLAUDE/settings.json",
            "/p/.codex/config.toml",
            "/p/.mcp.json",
            "/p/pkg/.mcp.json",
            "/p/.zed/tasks.json",
            "/p/.fyx/security.toml",
            "/p/.git/hooks/pre-commit",
            "/p/.Git/Hooks/post-checkout",
            "/p/src/../.claude/settings.json",
        ] {
            assert!(protected_config(Path::new(path)).is_some(), "{path}");
        }
        for path in [
            "/p/src/main.rs",
            "/p/claude/settings.json",
            "/p/.mcp.json.bak/x",
            "/p/.git/config",
            "/p/hooks/.git",
            "/p/docs/.mcp.json.md",
            "/p/.claude/../src/main.rs",
        ] {
            assert!(protected_config(Path::new(path)).is_none(), "{path}");
        }
        assert_eq!(
            protected_config_rel(rel_path("a/.git/hooks/pre-push")).as_deref(),
            Some(".git/hooks/")
        );
        assert_eq!(
            protected_config_rel(rel_path(".mcp.json")).as_deref(),
            Some(".mcp.json")
        );
    }

    #[test]
    fn repo_policy_only_adds_patterns() {
        let policy = RepoPolicy::parse(
            r#"
            private_files = ["config/credentials/**", "*.p12"]
            read_only = ["infra"]
            "#,
        )
        .unwrap();
        assert!(
            policy
                .private_match(rel_path("config/credentials/prod.json"))
                .is_some()
        );
        assert!(policy.private_match(rel_path("certs/client.p12")).is_some());
        assert!(policy.private_match(rel_path("config/app.json")).is_none());
        assert!(policy.read_only_match(rel_path("infra/main.tf")).is_some());
        assert!(policy.read_only_match(rel_path("src/infra.rs")).is_none());

        // Keys that would loosen or raise protection do not exist, so they make the file invalid.
        for text in [
            r#"mode = "off""#,
            r#"mode = "shield""#,
            r#"allow_env = ["GITHUB_TOKEN"]"#,
            r#"public_files = [".env"]"#,
            r#"private_files = ["["]"#,
            "private_files = ",
        ] {
            assert!(RepoPolicy::parse(text).is_err(), "{text}");
        }
        assert!(RepoPolicy::parse("").is_ok());
    }

    #[test]
    fn repo_state_fails_closed() {
        let path = rel_path("src/main.rs");
        assert!(RepoState::Loaded(None).read_denial(path).is_none());
        assert!(RepoState::Loaded(None).write_denial(path).is_none());
        assert!(RepoState::Loading.read_denial(path).is_some());
        assert!(RepoState::Loading.write_denial(path).is_some());
        let failed = RepoState::Failed("invalid TOML".into());
        assert!(failed.read_denial(path).unwrap().contains("invalid TOML"));
        assert!(failed.write_denial(path).is_some());

        let loaded = RepoState::Loaded(Some(
            RepoPolicy::parse(r#"read_only = ["Makefile"]"#).unwrap(),
        ));
        assert!(loaded.read_denial(rel_path("Makefile")).is_none());
        assert!(
            loaded
                .write_denial(rel_path("Makefile"))
                .unwrap()
                .contains("Makefile")
        );
    }

    #[test]
    fn env_patterns() {
        assert!(env_name_matches("AWS_*", "AWS_SECRET_ACCESS_KEY"));
        assert!(env_name_matches("AWS_*", "aws_profile"));
        assert!(!env_name_matches("AWS_*", "MY_AWS"));
        assert!(env_name_matches("*_API_KEY", "ANTHROPIC_API_KEY"));
        assert!(!env_name_matches("*_API_KEY", "API_KEY"));
        assert!(env_name_matches("*_SECRET*", "DJANGO_SECRET_KEY"));
        assert!(env_name_matches("*_SECRET*", "CLIENT_SECRET"));
        assert!(!env_name_matches("*_SECRET*", "SECRETARY"));
        assert!(env_name_matches("*_TOKEN", "CLAUDE_CODE_OAUTH_TOKEN"));
        assert!(!env_name_matches("*_TOKEN", "TOKENIZERS_PARALLELISM"));
        assert!(env_name_matches("GH_TOKEN", "gh_token"));
        assert!(!env_name_matches("GH_TOKEN", "GH_TOKENS"));
        assert!(env_name_matches("*", "ANYTHING"));
        assert!(env_name_matches("A*B*C", "AxxBxxC"));
        assert!(!env_name_matches("A*B*C", "AxxCxxB"));
        assert!(!env_name_matches("AB*BA", "ABA"));
    }

    #[test]
    fn secret_env_names_respect_the_allowlist() {
        let names = [
            "PATH",
            "HOME",
            "AWS_ACCESS_KEY_ID",
            "GITHUB_TOKEN",
            "GH_TOKEN",
            "NPM_TOKEN",
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_BASE_URL",
            "STRIPE_SECRET_KEY",
            "SSH_AUTH_SOCK",
            "TERM",
        ]
        .map(String::from);
        assert_eq!(
            secret_env_names(&names, &[]),
            [
                "ANTHROPIC_API_KEY",
                "ANTHROPIC_AUTH_TOKEN",
                "AWS_ACCESS_KEY_ID",
                "GH_TOKEN",
                "GITHUB_TOKEN",
                "NPM_TOKEN",
                "OPENAI_API_KEY",
                "SSH_AUTH_SOCK",
                "STRIPE_SECRET_KEY",
            ]
        );
        let allow = ["github_token".to_string(), "SSH_*".to_string()];
        let secret = secret_env_names(&names, &allow);
        assert!(!secret.contains(&"GITHUB_TOKEN".to_string()));
        assert!(!secret.contains(&"SSH_AUTH_SOCK".to_string()));
        assert!(secret.contains(&"GH_TOKEN".to_string()));
    }
}
