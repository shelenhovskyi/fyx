//! Paths that run code or hold secrets, shared by Fyx's agent shield (`fyx_guard`) and the
//! extension host: the same rule list protects agent configs from agents (under shield) and from
//! `workspace:edit` (always, integration-5 S2), and Zed's default `private_files` keep file text out
//! of `agent:observe` (integration-5 S1). Pure: path components in, the rule that matched out.

/// Directories agents and extensions may not write into, at any depth. All of `.git/`, not only
/// `hooks/`: `config` (`core.hooksPath`, `core.fsmonitor`), `info/`, a submodule's
/// `modules/*/config` and a linked worktree's `commondir` each make the next `git` run start a
/// program. A `.git` file redirects git to another directory, so the name is protected as a file
/// too.
///
/// Other agents' configs and Fyx's task sources are protected too (security-2 T2): `.gemini/`,
/// `.agents/` and `.cursor/` hold agent settings and hooks, and `.zed/` holds tasks.
pub const PROTECTED_DIRS: &[&str] = &[
    ".claude", ".codex", ".gemini", ".agents", ".cursor", ".zed", ".fyx", ".git",
];

/// Files agents and extensions may not write, at any depth: MCP server lists, and agent rules
/// files, which steer every later agent run (decided 2026-11-18: read-only under shield).
pub const PROTECTED_FILES: &[&str] = &[
    ".mcp.json",
    "AGENTS.md",
    "CLAUDE.md",
    "GEMINI.md",
    ".rules",
    ".cursorrules",
];

/// Files that may not be written inside a directory of this name, at any depth: VS Code's task
/// and launch configs run programs.
pub const PROTECTED_FILES_IN_DIR: &[(&str, &str)] =
    &[(".vscode", "tasks.json"), (".vscode", "launch.json")];

/// Names the agent-config rule `names` (the normal components of a path, in order) falls under,
/// as `dir/`, `file` or `dir/file`. ASCII case is ignored: case-insensitive file systems (macOS,
/// Windows) resolve `.Claude` to `.claude`.
pub fn protected_config_in<S: AsRef<str>>(names: &[S]) -> Option<String> {
    let is = |name: &str, protected: &str| name.eq_ignore_ascii_case(protected);
    for (index, name) in names.iter().enumerate() {
        let name = name.as_ref();
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
                .find(|(dir, file)| is(names[index - 1].as_ref(), dir) && is(name, file))
        {
            return Some(format!("{dir}/{file}"));
        }
    }
    None
}

/// Zed's default `private_files` (`assets/settings/default.json`): `**/.env*`, `**/*.pem`,
/// `**/*.key`, `**/*.cert`, `**/*.crt`, `**/secrets.yml`.
pub const DEFAULT_PRIVATE_FILES: &[&str] = &[
    "**/.env*",
    "**/*.pem",
    "**/*.key",
    "**/*.cert",
    "**/*.crt",
    "**/secrets.yml",
];

/// Whether a `/`-separated path matches [`DEFAULT_PRIVATE_FILES`], ignoring ASCII case. The host
/// applies the user's own `private_files` too; this is the floor when it cannot.
pub fn is_default_private(path: &str) -> bool {
    let Some(name) = path.rsplit('/').next() else {
        return false;
    };
    let name = name.to_ascii_lowercase();
    name.starts_with(".env")
        || [".pem", ".key", ".cert", ".crt"]
            .iter()
            .any(|suffix| name.ends_with(suffix))
        || name == "secrets.yml"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(path: &str) -> Option<String> {
        protected_config_in(&path.split('/').collect::<Vec<_>>())
    }

    #[test]
    fn configs_are_protected_at_any_depth() {
        for (path, matched) in [
            (".claude/settings.json", ".claude/"),
            ("pkg/.claude/settings.json", ".claude/"),
            ("sub/.zed/settings.json", ".zed/"),
            ("a/b/.git/hooks/pre-push", ".git/"),
            ("vendor/.git", ".git/"),
            (".mcp.json", ".mcp.json"),
            ("sub/.MCP.json", ".mcp.json"),
            ("docs/AGENTS.md", "AGENTS.md"),
            ("CLAUDE.md", "CLAUDE.md"),
            ("x/.vscode/tasks.json", ".vscode/tasks.json"),
            (".codex/config.toml", ".codex/"),
            (".cursor/rules", ".cursor/"),
        ] {
            assert_eq!(rule(path).as_deref(), Some(matched), "{path}");
        }
        for path in [
            "src/main.rs",
            "notes/mcp.json",
            "notes/settings.json",
            ".vscode/extensions.json",
            "docs/agents.md.bak",
            ".claudex/x",
        ] {
            assert_eq!(rule(path), None, "{path}");
        }
    }

    #[test]
    fn default_private_files() {
        for path in [
            ".env",
            "app/.env.local",
            "certs/server.PEM",
            "id.key",
            "a/b.cert",
            "c.crt",
            "config/secrets.yml",
        ] {
            assert!(is_default_private(path), "{path}");
        }
        for path in ["src/env.rs", "keys/readme.md", "secrets.yaml", "a.keystore"] {
            assert!(!is_default_private(path), "{path}");
        }
    }
}
