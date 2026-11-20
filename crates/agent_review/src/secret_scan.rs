//! Looks for secrets in the lines an agent adds (plan 7.3): known key formats and high-entropy
//! values assigned to secret-looking names. A finding holds the edit for review with a warning,
//! whatever the review mode, the user's `auto_accept` patterns or "accept all in turn" say.
//!
//! It is a tripwire for the common accident (a key pasted into a config or a test), not a guarantee:
//! an encoded or split secret passes.

use std::path::PathBuf;

use crate::ProposedEdit;

/// One secret-looking value in the new text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecretFinding {
    /// What it looks like, such as "AWS access key".
    pub kind: &'static str,
    /// The line of the new text, 1-based.
    pub line: u32,
    /// The value with its middle masked, safe to show and log.
    pub masked: String,
}

impl SecretFinding {
    pub fn describe(&self) -> String {
        format!("line {}: {} ({})", self.line, self.kind, self.masked)
    }
}

/// At most this many findings are kept per file.
const MAX_FINDINGS: usize = 8;
/// Lines longer than this (minified files, data) are skipped.
const MAX_LINE_BYTES: usize = 4096;

/// Known prefixes: (prefix, kind, minimum length of the rest, characters allowed in the rest).
const PREFIXES: &[(&str, &str, usize, fn(char) -> bool)] = &[
    ("AKIA", "AWS access key", 16, upper_or_digit),
    ("ASIA", "AWS temporary access key", 16, upper_or_digit),
    ("ghp_", "GitHub token", 36, alphanumeric),
    ("gho_", "GitHub OAuth token", 36, alphanumeric),
    ("ghu_", "GitHub token", 36, alphanumeric),
    ("ghs_", "GitHub token", 36, alphanumeric),
    ("ghr_", "GitHub refresh token", 36, alphanumeric),
    ("github_pat_", "GitHub token", 60, word),
    ("glpat-", "GitLab token", 20, token),
    ("xoxb-", "Slack token", 20, token),
    ("xoxp-", "Slack token", 20, token),
    ("xoxa-", "Slack token", 20, token),
    ("sk-ant-", "Anthropic API key", 32, token),
    ("sk-proj-", "OpenAI API key", 32, token),
    ("sk_live_", "Stripe secret key", 20, alphanumeric),
    ("rk_live_", "Stripe restricted key", 20, alphanumeric),
    ("AIza", "Google API key", 35, token),
    ("npm_", "npm token", 36, alphanumeric),
];

/// Names that make a long random-looking value assigned to them a secret.
const SECRET_NAMES: &[&str] = &[
    "secret",
    "token",
    "password",
    "passwd",
    "api_key",
    "apikey",
    "api-key",
    "access_key",
    "private_key",
    "client_secret",
    "auth",
    "credential",
];

/// Values that are obviously not real.
const PLACEHOLDERS: &[&str] = &[
    "example",
    "xxxx",
    "your",
    "changeme",
    "placeholder",
    "dummy",
    "redacted",
    "<",
    "${",
    "{{",
    "****",
    "0000",
    "1234",
    "abcd",
    "test",
];

fn upper_or_digit(character: char) -> bool {
    character.is_ascii_uppercase() || character.is_ascii_digit()
}

fn alphanumeric(character: char) -> bool {
    character.is_ascii_alphanumeric()
}

fn word(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_'
}

fn token(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_' || character == '-'
}

/// Secrets on the lines `edit` adds: its hunks' new text.
pub fn scan_edit(edit: &ProposedEdit) -> Vec<SecretFinding> {
    let mut findings = Vec::new();
    for hunk in edit.hunks() {
        let added = &edit.new_text()[hunk.new_bytes.clone()];
        for (offset, line) in (0u32..).zip(added.lines()) {
            if findings.len() >= MAX_FINDINGS {
                return findings;
            }
            if line.len() > MAX_LINE_BYTES {
                continue;
            }
            let line_number = hunk.new_rows.start.saturating_add(offset).saturating_add(1);
            findings.extend(
                scan_line(line)
                    .into_iter()
                    .map(|(kind, value)| SecretFinding {
                        kind,
                        line: line_number,
                        masked: mask(value),
                    }),
            );
        }
    }
    findings.truncate(MAX_FINDINGS);
    findings
}

/// Secrets on the lines `new_text` adds to `old_text`.
pub fn scan_added(old_text: &str, new_text: &str) -> Vec<SecretFinding> {
    scan_edit(&ProposedEdit::new(
        PathBuf::new(),
        old_text.to_string(),
        new_text.to_string(),
    ))
}

fn scan_line(line: &str) -> Vec<(&'static str, &str)> {
    let mut found = Vec::new();
    if line.contains("-----BEGIN") && line.contains("PRIVATE KEY-----") {
        let start = line.find("-----BEGIN").unwrap_or(0);
        found.push(("private key", line[start..].trim()));
        return found;
    }
    for (start, _) in line.char_indices() {
        // Only at the start of a token, so `xAKIA…` inside a longer word is not one.
        if line[..start]
            .chars()
            .next_back()
            .is_some_and(|previous| token(previous))
        {
            continue;
        }
        let rest = &line[start..];
        for (prefix, kind, minimum, allowed) in PREFIXES {
            let Some(after) = rest.strip_prefix(prefix) else {
                continue;
            };
            let length = after
                .find(|character| !allowed(character))
                .unwrap_or(after.len());
            let value = &rest[..prefix.len() + length];
            if length >= *minimum && !is_placeholder(value) {
                found.push((*kind, value));
            }
        }
    }
    if found.is_empty()
        && let Some(value) = assigned_secret(line)
    {
        found.push(("high-entropy value assigned to a secret name", value));
    }
    found
}

/// A quoted or bare value of at least 20 random-looking characters assigned (`=`, `:`) to a name
/// that sounds secret.
fn assigned_secret(line: &str) -> Option<&str> {
    // `=` first: in `const SECRET: &str = "…"` the value follows it, not the `:`.
    let separator = line.find('=').or_else(|| line.find(':'))?;
    let name = line[..separator].to_ascii_lowercase();
    if !SECRET_NAMES.iter().any(|secret| name.contains(secret)) {
        return None;
    }
    let value = line[separator + 1..]
        .trim_start_matches(['=', ' ', '\t'])
        .trim_start_matches(['"', '\'', '`']);
    let length = value
        .find(|character: char| !(token(character) || matches!(character, '+' | '/' | '=' | '.')))
        .unwrap_or(value.len());
    let value = &value[..length];
    let random = value.len() >= 20
        && !is_placeholder(value)
        && entropy(value) >= 3.5
        && value.chars().any(|character| character.is_ascii_digit())
        && value
            .chars()
            .any(|character| character.is_ascii_alphabetic());
    random.then_some(value)
}

fn is_placeholder(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    PLACEHOLDERS
        .iter()
        .any(|placeholder| lower.contains(placeholder))
}

/// Shannon entropy in bits per character.
fn entropy(value: &str) -> f64 {
    let mut counts = [0u32; 128];
    let mut total = 0u32;
    for byte in value.bytes().filter(u8::is_ascii) {
        counts[usize::from(byte)] += 1;
        total += 1;
    }
    if total == 0 {
        return 0.0;
    }
    let total = f64::from(total);
    counts
        .iter()
        .filter(|count| **count > 0)
        .map(|count| {
            let share = f64::from(*count) / total;
            -share * share.log2()
        })
        .sum()
}

/// The first four and last four characters, the rest masked.
fn mask(value: &str) -> String {
    let characters = value.chars().collect::<Vec<_>>();
    if characters.len() <= 12 {
        return "…".into();
    }
    let head = characters[..4].iter().collect::<String>();
    let tail = characters[characters.len() - 4..]
        .iter()
        .collect::<String>();
    format!("{head}…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_formats_are_found_on_added_lines_only() {
        let old = "region = \"eu-west-1\"\n";
        let new = "region = \"eu-west-1\"\naws_access_key_id = AKIAQ3EGRV7XKD2MBS5T\n";
        let findings = scan_added(old, new);
        assert_eq!(
            findings,
            vec![SecretFinding {
                kind: "AWS access key",
                line: 2,
                masked: "AKIA…BS5T".into(),
            }]
        );
        assert!(scan_added(new, new).is_empty(), "the line was there before");
        assert!(
            !findings[0].describe().contains("Q3EGRV7XKD2M"),
            "the value is never shown whole"
        );
    }

    #[test]
    fn tokens_and_keys() {
        let github = format!("token: ghp_{}", "a1B2c3D4e5".repeat(4));
        assert_eq!(scan_added("", &github)[0].kind, "GitHub token");
        let key = "-----BEGIN OPENSSH PRIVATE KEY-----";
        assert_eq!(scan_added("", key)[0].kind, "private key");
        let anthropic = format!(
            "ANTHROPIC_API_KEY=sk-ant-api03-{}",
            "Zx9_Qw8-Er7Ty6Ui".repeat(3)
        );
        assert_eq!(scan_added("", &anthropic)[0].kind, "Anthropic API key");
    }

    #[test]
    fn random_values_assigned_to_secret_names() {
        let line = r#"const CLIENT_SECRET: &str = "q8Zr3LmX0vT7pW2nY5kB9dGs";"#;
        let findings = scan_added("", line);
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].kind,
            "high-entropy value assigned to a secret name"
        );
    }

    #[test]
    fn ordinary_code_and_placeholders_pass() {
        for line in [
            "let token = tokenizer.next_token();",
            "password = \"your-password-here-please-change\"",
            "AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE",
            "api_key = os.environ[\"API_KEY\"]",
            "secret_name = \"database-connection-string\"",
            "let hash = \"3f786850e387550fdab836ed7e6dc881de23001b\";",
            "MAKIAQ3EGRV7XKD2MBS5T",
        ] {
            assert!(scan_added("", line).is_empty(), "{line}");
        }
    }

    #[test]
    fn entropy_and_masking() {
        assert!(entropy("aaaaaaaaaaaaaaaaaaaa") < 1.0);
        assert!(entropy("q8Zr3LmX0vT7pW2nY5kB") > 4.0);
        assert_eq!(mask("short"), "…");
    }
}
