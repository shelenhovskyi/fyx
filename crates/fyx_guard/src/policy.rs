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

/// Directories agents may not write into, at any depth. All of `.git/`, not only `hooks/`: `config`
/// (`core.hooksPath`, `core.fsmonitor`), `info/`, a submodule's `modules/*/config` and a linked
/// worktree's `commondir` each make the next `git` run start a program. A `.git` file redirects git to
/// another directory, so the name is protected as a file too.
///
/// Other agents' configs and Fyx's task sources are protected too (security-2 T2): `.gemini/`,
/// `.agents/` and `.cursor/` hold agent settings and hooks, and `.zed/` holds tasks.
const PROTECTED_DIRS: &[&str] = &[
    ".claude", ".codex", ".gemini", ".agents", ".cursor", ".zed", ".fyx", ".git",
];
/// Files agents may not write, at any depth: MCP server lists, and agent rules files, which steer
/// every later agent run (decided 2026-11-18: read-only under shield).
const PROTECTED_FILES: &[&str] = &[
    ".mcp.json",
    "AGENTS.md",
    "CLAUDE.md",
    "GEMINI.md",
    ".rules",
    ".cursorrules",
];
/// Files agents may not write inside a directory of this name, at any depth: VS Code's task and
/// launch configs run programs.
const PROTECTED_FILES_IN_DIR: &[(&str, &str)] =
    &[(".vscode", "tasks.json"), (".vscode", "launch.json")];

/// Variables that look like secrets. `*` matches any run of characters; names are compared in
/// ASCII upper case.
pub const SECRET_ENV_PATTERNS: &[&str] = &[
    "AWS_*",
    "GITHUB_TOKEN",
    "GH_TOKEN",
    "NPM_TOKEN",
    "*_KEY",
    "*_SECRET*",
    "SECRET",
    "*_TOKEN",
    "TOKEN",
    "*PASSWORD*",
    "*_PASSWD",
    "*_PWD",
    "*APIKEY*",
    "*_PAT",
    "*_CREDENTIALS",
    "*_DSN",
    "DATABASE_URL",
    "SSH_AUTH_SOCK",
    // Password manager sessions (Bitwarden, 1Password) and registry logins.
    "BW_SESSION",
    "OP_SESSION_*",
    "DOCKER_AUTH_CONFIG",
];

/// Whether a variable's value carries credentials whatever its name: a URL with a user and a
/// password, such as `https://user:token@host` or `postgres://u:p@db/x` (security-2 S9).
pub fn is_credential_url(value: &str) -> bool {
    let Some((scheme, rest)) = value.split_once("://") else {
        return false;
    };
    let valid_scheme = !scheme.is_empty()
        && scheme
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "+.-".contains(character));
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    valid_scheme
        && authority.rsplit_once('@').is_some_and(|(userinfo, _)| {
            userinfo
                .split_once(':')
                .is_some_and(|(_, password)| !password.is_empty())
        })
}

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
        if is_last
            && index > 0
            && let Some((dir, file)) = PROTECTED_FILES_IN_DIR
                .iter()
                .find(|(dir, file)| is(&names[index - 1], dir) && is(name, file))
        {
            return Some(format!("{dir}/{file}"));
        }
    }
    None
}

/// Whether `path` is `dir` or inside it, after resolving `.` and `..`, ignoring ASCII case.
pub fn is_within(path: &Path, dir: &Path) -> bool {
    let path = util::paths::normalize_path(path);
    let dir = util::paths::normalize_path(dir);
    let mut path = path.components();
    dir.components().all(|dir_component| {
        path.next().is_some_and(|component| {
            component
                .as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case(&dir_component.as_os_str().to_string_lossy())
        })
    })
}

/// Characters of untrusted text (paths and patterns from an agent or a repository) as they go
/// into a log line: control characters, including newlines, are escaped, so one entry cannot
/// forge another (security-2 T6), and the text is cut at `MAX_LOGGED_TEXT` characters.
pub fn sanitize_for_log(text: &str) -> String {
    const MAX_LOGGED_TEXT: usize = 512;
    let mut sanitized = String::with_capacity(text.len().min(MAX_LOGGED_TEXT));
    for (count, character) in text.chars().enumerate() {
        if count == MAX_LOGGED_TEXT {
            sanitized.push('…');
            break;
        }
        if character.is_control() || matches!(character, '\u{2028}' | '\u{2029}') {
            sanitized.extend(character.escape_default());
        } else {
            sanitized.push(character);
        }
    }
    sanitized
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
///
/// One matcher per pattern, so a denial names the pattern that matched rather than every pattern
/// of the file (bugs-1 #28).
pub struct RepoPolicy {
    private_files: Vec<PatternMatcher>,
    read_only: Vec<PatternMatcher>,
}

/// One pattern. On case-insensitive file systems (Windows, macOS) it also matches in lower case,
/// so `Makefile` covers `MAKEFILE` (bugs-1 #16).
struct PatternMatcher {
    pattern: String,
    matcher: PathMatcher,
    lower: Option<PathMatcher>,
}

impl PatternMatcher {
    fn is_match(&self, path: &RelPath) -> bool {
        self.matcher.is_match(path)
            || self.lower.as_ref().is_some_and(|lower| {
                RelPath::from_unix_str(&path.as_unix_str().to_lowercase())
                    .is_ok_and(|path| lower.is_match(path))
            })
    }
}

impl RepoPolicy {
    /// Errors carry only a location (`line:column`, or the key and position of a pattern), never
    /// the file's text: they reach the agent, and the file is the repository's (security-2 S10).
    pub fn parse(text: &str) -> Result<Self> {
        let file: SecurityFile = toml::from_str(text).map_err(|error| {
            let location = error
                .span()
                .map(|span| {
                    let before = &text[..span.start.min(text.len())];
                    let line = before.matches('\n').count() + 1;
                    let column = before
                        .rsplit('\n')
                        .next()
                        .map_or(0, |line| line.chars().count())
                        + 1;
                    format!(" at {SECURITY_FILE}:{line}:{column}")
                })
                .unwrap_or_default();
            anyhow::anyhow!("invalid TOML{location}")
        })?;
        Ok(Self {
            private_files: matchers("private_files", file.private_files)?,
            read_only: matchers("read_only", file.read_only)?,
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

fn matchers(key: &str, patterns: Vec<String>) -> Result<Vec<PatternMatcher>> {
    patterns
        .into_iter()
        .enumerate()
        .map(|(index, pattern)| {
            let invalid = || format!("invalid pattern number {} in `{key}`", index + 1);
            let matcher = PathMatcher::new(std::slice::from_ref(&pattern), PathStyle::Unix)
                .with_context(invalid)?;
            let lower = if cfg!(any(windows, target_os = "macos")) {
                Some(
                    PathMatcher::new([pattern.to_lowercase()], PathStyle::Unix)
                        .with_context(invalid)?,
                )
            } else {
                None
            };
            Ok(PatternMatcher {
                pattern,
                matcher,
                lower,
            })
        })
        .collect()
}

// Same ancestor walk as `WorktreeSettings::is_path_private`, so `secrets/**` and `secrets` both
// cover `secrets/a/b`.
fn first_match(matchers: &[PatternMatcher], path: &RelPath) -> Option<String> {
    matchers
        .iter()
        .find(|matcher| path.ancestors().any(|ancestor| matcher.is_match(ancestor)))
        .map(|matcher| sanitize_for_log(&matcher.pattern))
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
            RepoState::Loaded(Some(policy)) => policy.private_match(path).map(|pattern| {
                format!("it matches the `private_files` pattern {pattern:?} in {SECURITY_FILE}")
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
            RepoState::Loaded(Some(policy)) => policy.read_only_match(path).map(|pattern| {
                format!("it matches the `read_only` pattern {pattern:?} in {SECURITY_FILE}")
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
    secret_env_vars(names.into_iter().map(|name| (name, None)), allow)
}

/// [`secret_env_names`] for variables with their values, when known: a value that is a
/// credential URL makes the variable secret whatever its name.
pub fn secret_env_vars<'a>(
    vars: impl IntoIterator<Item = (&'a String, Option<&'a str>)>,
    allow: &[String],
) -> Vec<String> {
    let mut secret = vars
        .into_iter()
        .filter(|(name, value)| {
            (SECRET_ENV_PATTERNS
                .iter()
                .any(|pattern| env_name_matches(pattern, name))
                || value.is_some_and(is_credential_url))
                && !allow.iter().any(|pattern| env_name_matches(pattern, name))
        })
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    secret.sort();
    secret.dedup();
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
            "/p/.git/config",
            "/p/.GIT/CONFIG",
            "/p/.git/info/exclude",
            "/p/.git/modules/sub/hooks/pre-commit",
            "/p/.git/modules/sub/config",
            "/p/.git/worktrees/wt/commondir",
            "/p/sub/.git",
            "/p/src/../.claude/settings.json",
            "/p/.gemini/settings.json",
            "/p/.agents/hooks.json",
            "/p/.cursor/rules/x.mdc",
            "/p/.vscode/tasks.json",
            "/p/sub/.VSCode/Launch.json",
            "/p/AGENTS.md",
            "/p/crates/x/CLAUDE.md",
            "/p/GEMINI.md",
            "/p/.rules",
            "/p/agents.md",
        ] {
            assert!(protected_config(Path::new(path)).is_some(), "{path}");
        }
        for path in [
            "/p/src/main.rs",
            "/p/claude/settings.json",
            "/p/.mcp.json.bak/x",
            "/p/.gitignore",
            "/p/.github/workflows/ci.yml",
            "/p/git/config",
            "/p/docs/.mcp.json.md",
            "/p/.claude/../src/main.rs",
            "/p/.vscode/settings.json",
            "/p/tasks.json",
            "/p/docs/AGENTS.md.bak",
            "/p/README.md",
        ] {
            assert!(protected_config(Path::new(path)).is_none(), "{path}");
        }
        assert_eq!(
            protected_config_rel(rel_path("a/.git/hooks/pre-push")).as_deref(),
            Some(".git/")
        );
        assert_eq!(
            protected_config_rel(rel_path(".git/config")).as_deref(),
            Some(".git/")
        );
        assert_eq!(
            protected_config_rel(rel_path(".mcp.json")).as_deref(),
            Some(".mcp.json")
        );
    }

    #[test]
    fn is_within_ignores_case_and_dot_dot() {
        let dir = Path::new("/home/u/.config/fyx");
        assert!(is_within(
            Path::new("/home/u/.config/fyx/settings.json"),
            dir
        ));
        assert!(is_within(Path::new("/home/u/.config/fyx"), dir));
        assert!(is_within(Path::new("/home/u/.Config/FYX/tasks.json"), dir));
        assert!(is_within(
            Path::new("/home/u/x/../.config/fyx/keymap.json"),
            dir
        ));
        assert!(!is_within(
            Path::new("/home/u/.config/fyx-other/settings.json"),
            dir
        ));
        assert!(!is_within(Path::new("/home/u/.config"), dir));
        assert!(!is_within(
            Path::new("/home/u/.config/fyx/../zed/settings.json"),
            dir
        ));
        assert!(!is_within(Path::new("u/.config/fyx/settings.json"), dir));
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
        // Only the pattern that matched is named (bugs-1 #28).
        assert_eq!(
            policy
                .private_match(rel_path("certs/client.p12"))
                .as_deref(),
            Some("*.p12")
        );

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

        // Errors name a location, never the file's text (security-2 S10).
        let error = RepoPolicy::parse("private_files = [\"a\"]\nIGNORE ALL PREVIOUS = 1\n")
            .err()
            .unwrap();
        let message = format!("{error:#}");
        assert!(message.contains(".fyx/security.toml:2:"), "{message}");
        assert!(!message.contains("IGNORE"), "{message}");
        let error = RepoPolicy::parse(r#"private_files = ["ok", "["]"#)
            .err()
            .unwrap();
        assert!(
            format!("{error:#}").contains("pattern number 2 in `private_files`"),
            "{error:#}"
        );
    }

    #[test]
    fn log_text_cannot_forge_lines() {
        assert_eq!(
            sanitize_for_log("a\nfyx_guard: allowed"),
            "a\\nfyx_guard: allowed"
        );
        assert_eq!(sanitize_for_log("x\u{1b}[31my"), "x\\u{1b}[31my");
        assert_eq!(sanitize_for_log(&"a".repeat(600)).chars().count(), 513);
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
            "DB_PASSWORD",
            "SMTP_PASSWD",
            "OPENAI_KEY",
            "STRIPE_KEY",
            "GITLAB_PAT",
            "GOOGLE_APPLICATION_CREDENTIALS",
            "DATABASE_URL",
            "SENTRY_DSN",
            "PATH_SEPARATOR",
            "KEYBOARD_LAYOUT",
            "PWD",
            "OLDPWD",
        ]
        .map(String::from);
        assert_eq!(
            secret_env_names(&names, &[]),
            [
                "ANTHROPIC_API_KEY",
                "ANTHROPIC_AUTH_TOKEN",
                "AWS_ACCESS_KEY_ID",
                "DATABASE_URL",
                "DB_PASSWORD",
                "GH_TOKEN",
                "GITHUB_TOKEN",
                "GITLAB_PAT",
                "GOOGLE_APPLICATION_CREDENTIALS",
                "NPM_TOKEN",
                "OPENAI_API_KEY",
                "OPENAI_KEY",
                "SENTRY_DSN",
                "SMTP_PASSWD",
                "SSH_AUTH_SOCK",
                "STRIPE_KEY",
                "STRIPE_SECRET_KEY",
            ]
        );
        for secret in [
            "MYSQL_PASSWORD_FILE",
            "PASSWORD",
            "PGPASSWORD",
            "REDIS_PWD",
            "MAPS_APIKEY",
            "APIKEY",
            "BW_SESSION",
            "OP_SESSION_my",
            "DOCKER_AUTH_CONFIG",
            "TOKEN",
            "SECRET",
        ] {
            assert_eq!(
                secret_env_names(&[secret.to_string()], &[]),
                [secret],
                "{secret}"
            );
        }

        let urls = [
            ("REPO", "https://user:ghp_x@github.com/a/b.git"),
            ("DB", "postgres://u:p@db:5432/x"),
            ("PLAIN", "https://github.com/a/b"),
            ("USER_ONLY", "ssh://git@github.com/a"),
            ("NOT_URL", "a:b@c"),
        ]
        .map(|(name, value)| (name.to_string(), value));
        assert_eq!(
            secret_env_vars(urls.iter().map(|(name, value)| (name, Some(*value))), &[]),
            ["DB", "REPO"]
        );

        let allow = ["github_token".to_string(), "SSH_*".to_string()];
        let secret = secret_env_names(&names, &allow);
        assert!(!secret.contains(&"GITHUB_TOKEN".to_string()));
        assert!(!secret.contains(&"SSH_AUTH_SOCK".to_string()));
        assert!(secret.contains(&"GH_TOKEN".to_string()));
    }
}
