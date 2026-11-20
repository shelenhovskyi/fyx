//! zsh modifiers in the commands Claude's Bash tool runs.
//!
//! Claude Code runs its Bash tool in `CLAUDE_CODE_SHELL`, else in `SHELL` when that is bash or
//! zsh (read from Claude Code 2.1.329). On macOS that is usually zsh, and zsh reads a colon after
//! an unbraced parameter as a history modifier, inside double quotes too: `"$M:refs/heads/dev"`
//! is `$M` without its extension (`:r`) followed by `efs/heads/dev`, with no error. The model
//! writes such strings out of bash habit (refspecs, `host:path`, `image:$TAG`), so the `pre-tool`
//! hook denies a command with one and says how to write it ([`deny_answer`]); the braced forms
//! `${M}:` and `${M:r}` are never flagged.

/// zsh's modifiers (`zshexpn(1)`, "Modifiers"). `:s` and `:S` take a delimiter, so `$X:src`
/// is a substitution; `:g` and `:&` repeat one. Every other letter or a digit after the colon
/// is a literal colon.
const MODIFIERS: &str = "aAcehlPqQrsStuxg&";

/// One `$NAME:m` that zsh would read as a modifier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hazard {
    /// The parameter, without `$`.
    pub parameter: String,
    pub modifier: char,
}

impl Hazard {
    /// `$M:r`.
    pub fn source(&self) -> String {
        format!("${}:{}", self.parameter, self.modifier)
    }
}

/// Whether the shell Claude Code picks is zsh. `candidates` are `CLAUDE_CODE_SHELL` and `SHELL`
/// in that order (empty when unset); like Claude Code, only a path that names bash or zsh
/// counts. With neither, Claude Code probes the system's shells, which the hook cannot see, so
/// this says no and the hook lets the command run.
pub fn shell_is_zsh<'a>(candidates: impl IntoIterator<Item = &'a str>) -> bool {
    candidates
        .into_iter()
        .map(|path| {
            path.rsplit(['/', '\\'])
                .next()
                .unwrap_or(path)
                .trim_end_matches(".exe")
        })
        .find(|name| name.contains("bash") || name.contains("zsh"))
        .is_some_and(|name| name.contains("zsh"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Context {
    /// Outside quotes, and inside `$(...)` (with its parenthesis depth).
    Plain {
        parentheses: usize,
    },
    Double,
    Backtick,
}

struct Heredoc {
    delimiter: String,
    strip_tabs: bool,
    /// `<<'EOF'`, `<<"EOF"`, `<<\EOF`: the body is not expanded.
    quoted: bool,
}

/// Every unbraced `$NAME:m` that zsh expands, in order: outside quotes, in double quotes, in
/// backticks and `$(...)`, and in the body of a heredoc whose delimiter is not quoted. Single
/// quotes, `$'...'`, backslash escapes, comments and quoted heredocs are skipped.
pub fn hazards(command: &str) -> Vec<Hazard> {
    let chars: Vec<char> = command.chars().collect();
    let mut found = Vec::new();
    let mut stack = vec![Context::Plain { parentheses: 0 }];
    let mut heredocs: Vec<Heredoc> = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let character = chars[index];
        let context = stack
            .last()
            .copied()
            .unwrap_or(Context::Plain { parentheses: 0 });
        let next = chars.get(index + 1).copied();
        match (context, character) {
            (_, '\\') => index += 2,
            (_, '$') => {
                match next {
                    Some('(') => {
                        stack.push(Context::Plain { parentheses: 1 });
                        index += 2;
                    }
                    Some('\'') if context != Context::Double => {
                        index = skip_ansi_c(&chars, index + 2);
                    }
                    _ => {
                        let (hazard, end) = parameter(&chars, index + 1);
                        found.extend(hazard);
                        index = end;
                    }
                }
                continue;
            }
            (Context::Double, '"') | (Context::Backtick, '`') => {
                stack.pop();
                index += 1;
            }
            (Context::Double, _) | (Context::Backtick, _) => index += 1,
            (Context::Plain { .. }, '`') => {
                stack.push(Context::Backtick);
                index += 1;
            }
            (Context::Plain { .. }, '"') => {
                stack.push(Context::Double);
                index += 1;
            }
            (Context::Plain { .. }, '\'') => {
                index = chars[index + 1..]
                    .iter()
                    .position(|&character| character == '\'')
                    .map_or(chars.len(), |offset| index + offset + 2);
            }
            (Context::Plain { parentheses }, '(') => {
                if parentheses > 0
                    && let Some(top) = stack.last_mut()
                {
                    *top = Context::Plain {
                        parentheses: parentheses + 1,
                    };
                }
                index += 1;
            }
            (Context::Plain { parentheses }, ')') => {
                if parentheses == 1 {
                    stack.pop();
                } else if parentheses > 1
                    && let Some(top) = stack.last_mut()
                {
                    *top = Context::Plain {
                        parentheses: parentheses - 1,
                    };
                }
                index += 1;
            }
            (Context::Plain { .. }, '#') if starts_word(&chars, index) => {
                index = chars[index..]
                    .iter()
                    .position(|&character| character == '\n')
                    .map_or(chars.len(), |offset| index + offset);
            }
            // `<<<` is a here-string, not a heredoc.
            (Context::Plain { .. }, '<') if next == Some('<') => {
                if chars.get(index + 2) == Some(&'<') {
                    index += 3;
                } else {
                    let (heredoc, end) = heredoc_operator(&chars, index + 2);
                    heredocs.extend(heredoc);
                    index = end;
                }
            }
            (Context::Plain { .. }, '\n') if !heredocs.is_empty() => {
                index += 1;
                for heredoc in std::mem::take(&mut heredocs) {
                    index = heredoc_body(&chars, index, &heredoc, &mut found);
                }
            }
            (Context::Plain { .. }, _) => index += 1,
        }
    }
    found
}

/// After `$`: a name or one digit, then `:` and a modifier.
fn parameter(chars: &[char], start: usize) -> (Option<Hazard>, usize) {
    let mut end = start;
    match chars.get(start) {
        Some(character) if character.is_ascii_digit() => end += 1,
        Some(character) if character.is_ascii_alphabetic() || *character == '_' => {
            while chars
                .get(end)
                .is_some_and(|character| character.is_ascii_alphanumeric() || *character == '_')
            {
                end += 1;
            }
        }
        _ => return (None, start),
    }
    match (chars.get(end), chars.get(end + 1)) {
        (Some(':'), Some(&modifier)) if MODIFIERS.contains(modifier) => (
            Some(Hazard {
                parameter: chars[start..end].iter().collect(),
                modifier,
            }),
            end + 2,
        ),
        _ => (None, end),
    }
}

/// Past the closing quote of `$'...'`, whose backslashes escape.
fn skip_ansi_c(chars: &[char], mut index: usize) -> usize {
    while let Some(&character) = chars.get(index) {
        match character {
            '\\' => index += 2,
            '\'' => return index + 1,
            _ => index += 1,
        }
    }
    chars.len()
}

/// A `#` starts a comment at the start of a word.
fn starts_word(chars: &[char], index: usize) -> bool {
    index == 0
        || chars[index - 1].is_whitespace()
        || matches!(chars[index - 1], ';' | '&' | '|' | '(')
}

/// After `<<`: `-`, blanks and the delimiter word. `<<<` (a here-string) is not one.
fn heredoc_operator(chars: &[char], mut index: usize) -> (Option<Heredoc>, usize) {
    let strip_tabs = chars.get(index) == Some(&'-');
    if strip_tabs {
        index += 1;
    }
    while chars
        .get(index)
        .is_some_and(|&character| character == ' ' || character == '\t')
    {
        index += 1;
    }
    let mut delimiter = String::new();
    let mut quoted = false;
    while let Some(&character) = chars.get(index) {
        match character {
            '\'' | '"' => {
                quoted = true;
                let close = chars[index + 1..]
                    .iter()
                    .position(|&other| other == character)
                    .map_or(chars.len(), |offset| index + 1 + offset);
                delimiter.extend(&chars[index + 1..close.min(chars.len())]);
                index = close + 1;
            }
            '\\' => {
                quoted = true;
                delimiter.extend(chars.get(index + 1));
                index += 2;
            }
            character if character.is_whitespace() || ";&|<>()".contains(character) => break,
            character => {
                delimiter.push(character);
                index += 1;
            }
        }
    }
    let heredoc = (!delimiter.is_empty()).then_some(Heredoc {
        delimiter,
        strip_tabs,
        quoted,
    });
    (heredoc, index.min(chars.len()))
}

/// The body's lines up to the delimiter line; an unquoted body expands parameters (and quotes in
/// it are plain characters). Returns where the line after the delimiter starts.
fn heredoc_body(
    chars: &[char],
    mut index: usize,
    heredoc: &Heredoc,
    found: &mut Vec<Hazard>,
) -> usize {
    while index < chars.len() {
        let end = chars[index..]
            .iter()
            .position(|&character| character == '\n')
            .map_or(chars.len(), |offset| index + offset);
        let line: String = chars[index..end].iter().collect();
        let line = if heredoc.strip_tabs {
            line.trim_start_matches('\t')
        } else {
            line.as_str()
        };
        if line == heredoc.delimiter {
            return end;
        }
        if !heredoc.quoted {
            let mut position = index;
            while position < end {
                match chars[position] {
                    '\\' => position += 2,
                    '$' => {
                        let (hazard, next) = parameter(chars, position + 1);
                        found.extend(hazard);
                        position = next.max(position + 1);
                    }
                    _ => position += 1,
                }
            }
        }
        index = end + 1;
    }
    chars.len()
}

/// The `PreToolUse` answer that stops a command with [`hazards`]: exit 0 and a `deny` decision,
/// whose reason Claude reads.
pub fn deny_answer(hazards: &[Hazard]) -> Option<String> {
    const SHOWN: usize = 4;
    let first = hazards.first()?;
    let mut listed: Vec<String> = Vec::new();
    for hazard in hazards {
        let source = format!("`{}`", hazard.source());
        if !listed.contains(&source) {
            listed.push(source);
        }
    }
    let more = listed.len().saturating_sub(SHOWN);
    listed.truncate(SHOWN);
    let mut list = listed.join(", ");
    if more > 0 {
        list.push_str(&format!(" and {more} more"));
    }
    let reason = format!(
        "Not run: your Bash tool runs zsh, which reads {list} as a history modifier of the \
         parameter, not a colon, even inside double quotes, and silently changes the string. \
         Write `${{{name}}}:` for a literal colon (or `${{{name}:{modifier}}}` if the modifier \
         is meant), then run the command again.",
        name = first.parameter,
        modifier = first.modifier,
    );
    let answer = serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": reason,
        }
    });
    Some(answer.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sources(command: &str) -> Vec<String> {
        hazards(command).iter().map(Hazard::source).collect()
    }

    /// The cases zsh 5.9 changed in the report from the owner's Mac, and those it left alone.
    #[test]
    fn modifiers_after_unbraced_parameters_are_found() {
        assert_eq!(
            sources(r#"M=$(git commit-tree x) && git push origin "$M:refs/heads/dev""#),
            ["$M:r"]
        );
        assert_eq!(sources(r#"docker run -v "$P:app" img"#), ["$P:a"]);
        assert_eq!(sources(r#"docker tag x "$T:latest""#), ["$T:l"]);
        assert_eq!(sources(r#"scp file $HOST:tmp/"#), ["$HOST:t"]);
        assert_eq!(sources(r#"echo $1:h $_x9:e"#), ["$1:h", "$_x9:e"]);
        assert_eq!(sources("echo \"$(basename \"$F:t\")\""), ["$F:t"]);
        assert_eq!(sources("echo `echo $F:u`"), ["$F:u"]);

        for fine in [
            r#"curl "$T:8080" "$P:/app" "$HOST:$PORT""#,
            r#"git push origin "${M}:refs/heads/dev""#,
            r#"echo "${M:r}" ${F:-default}"#,
            r#"echo '$M:refs' $'it\'s $M:r'"#,
            r#"echo \$M:r "\$M:r""#,
            "# push $M:refs\necho ok",
            "echo $$:r $?:r",
            r#"cat <<< "${M}:r""#,
        ] {
            assert_eq!(sources(fine), Vec::<String>::new(), "{fine}");
        }
    }

    #[test]
    fn heredoc_bodies_expand_unless_the_delimiter_is_quoted() {
        assert_eq!(
            sources("cat <<EOF > f\nit's $HOST:tmp\nEOF\necho $M:r"),
            ["$HOST:t", "$M:r"]
        );
        assert_eq!(
            sources("cat <<'EOF' > f\nscp a \"$HOST:tmp\"\nEOF\necho done"),
            Vec::<String>::new()
        );
        assert_eq!(
            sources("cat <<-\"END\"\n\t$M:r\n\tEND\necho $T:l"),
            ["$T:l"]
        );
    }

    #[test]
    fn the_shell_is_the_first_bash_or_zsh_candidate() {
        assert!(shell_is_zsh(["", "/bin/zsh"]));
        assert!(shell_is_zsh(["/opt/homebrew/bin/zsh", "/bin/bash"]));
        assert!(!shell_is_zsh(["/bin/bash", "/bin/zsh"]));
        assert!(!shell_is_zsh(["/usr/bin/fish", "/usr/bin/bash"]));
        assert!(!shell_is_zsh(["C:\\Program Files\\Git\\bin\\bash.exe", ""]));
        assert!(!shell_is_zsh(["", ""]));
    }

    #[test]
    fn the_answer_denies_with_the_braced_spelling() {
        let answer: serde_json::Value = serde_json::from_str(
            &deny_answer(&hazards(
                r#"git push origin "$M:refs/heads/dev" "$M:refs/tags/x""#,
            ))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(deny_answer(&[]), None);
        let output = &answer["hookSpecificOutput"];
        assert_eq!(output["hookEventName"], "PreToolUse");
        assert_eq!(output["permissionDecision"], "deny");
        let reason = output["permissionDecisionReason"].as_str().unwrap();
        assert!(reason.contains("`$M:r` as a history modifier"), "{reason}");
        assert!(reason.contains("Write `${M}:`"), "{reason}");
        assert!(reason.contains("`${M:r}`"), "{reason}");
    }
}
