//! Filling a discovery-file template (design wit-0.4 §B.8): the typed JSON content, the folder
//! the file goes in, and its name. Pure: no file system.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

/// What the placeholders stand for, for one listener in this run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Vars {
    pub port: u16,
    pub token: String,
    pub pid: u32,
    /// The open windows' local worktree roots.
    pub worktrees: Vec<String>,
    /// `None` on Windows.
    pub uid: Option<u32>,
    pub tmp: String,
    pub windows: bool,
}

impl Vars {
    fn joined(&self) -> String {
        self.worktrees.join(if self.windows { ";" } else { ":" })
    }

    /// A placeholder's value as JSON, typed.
    fn typed(&self, name: &str) -> Result<Value, String> {
        Ok(match name {
            "port" => Value::from(self.port),
            "pid" => Value::from(self.pid),
            "token" => Value::from(self.token.clone()),
            "worktrees" => Value::from(self.worktrees.clone()),
            "worktrees.joined" => Value::from(self.joined()),
            "platform.windows" => Value::from(self.windows),
            "uid" => Value::from(self.uid.ok_or("${uid} has no value on Windows")?),
            "tmp" => Value::from(self.tmp.clone()),
            _ => return Err(format!("unknown placeholder ${{{name}}}")),
        })
    }

    /// A placeholder's value inside a longer string.
    fn text(&self, name: &str) -> Result<String, String> {
        Ok(match self.typed(name)? {
            Value::String(text) => text,
            // `${worktrees}` in text is the joined form; numbers and booleans as JSON writes them.
            Value::Array(_) => self.joined(),
            other => other.to_string(),
        })
    }
}

/// `text` with each `${name}` replaced by `value(name)`.
fn substitute(
    text: &str,
    mut value: impl FnMut(&str) -> Result<String, String>,
) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after
            .find('}')
            .ok_or_else(|| format!("{text:?}: an unclosed ${{"))?;
        out.push_str(&value(&after[..end])?);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// The content with its placeholders filled. A string that is exactly one placeholder takes that
/// placeholder's type; one inside a longer string is text (JSON escaping is the serializer's).
/// Objects and arrays are filled inside; other values stand as they are.
pub(crate) fn fill(content: &Value, vars: &Vars) -> Result<Value, String> {
    Ok(match content {
        Value::String(text) => match whole_placeholder(text) {
            Some(name) => vars.typed(name)?,
            None => Value::String(substitute(text, |name| vars.text(name))?),
        },
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| fill(item, vars))
                .collect::<Result<_, _>>()?,
        ),
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, value)| Ok((key.clone(), fill(value, vars)?)))
                .collect::<Result<Map<_, _>, String>>()?,
        ),
        other => other.clone(),
    })
}

fn whole_placeholder(text: &str) -> Option<&str> {
    let name = text.strip_prefix("${")?.strip_suffix('}')?;
    (!name.contains(['{', '}', '$'])).then_some(name)
}

/// The file name: `${port}` and `${pid}` filled, ending in `.lock` or `.json`, one plain name.
pub(crate) fn file_name(template: &str, vars: &Vars) -> Result<String, String> {
    if !template.contains("${port}") && !template.contains("${pid}") {
        return Err(format!("{template:?} must contain ${{port}} or ${{pid}}"));
    }
    let name = substitute(template, |name| match name {
        "port" => Ok(vars.port.to_string()),
        "pid" => Ok(vars.pid.to_string()),
        _ => Err(format!(
            "{template:?}: only ${{port}} and ${{pid}} go in a file name"
        )),
    })?;
    if !(name.ends_with(".lock") || name.ends_with(".json")) {
        return Err(format!("{name:?} must end in .lock or .json"));
    }
    if name.starts_with('.') || !is_plain_component(&name) {
        return Err(format!("{name:?} is not a plain file name"));
    }
    Ok(name)
}

fn is_plain_component(part: &str) -> bool {
    !part.is_empty()
        && part != "."
        && part != ".."
        && !part.contains(['/', '\\', ':', '\0'])
        && !part.contains("${")
}

/// A folder to write in: a trusted base (an account dir, the home folder or the temp folder) and
/// the plain folder names under it, which the writer checks one by one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Target {
    pub base: PathBuf,
    pub folders: Vec<String>,
}

/// Where `dir` points: one target per account dir of `${account.<tool>}`'s tool, else
/// `default_dir` under the home folder, or `${tmp}`. No targets means no file (an account dir
/// with no account and no default).
pub(crate) fn targets(
    dir: &str,
    default_dir: Option<&str>,
    account_dirs: impl Fn(&str) -> Vec<PathBuf>,
    home: Option<&Path>,
    tmp: &Path,
) -> Result<Vec<Target>, String> {
    let (head, rest) = dir.split_once('/').unwrap_or((dir, ""));
    let folders = folder_names(rest)?;
    if head == "${tmp}" {
        return Ok(vec![Target {
            base: tmp.to_path_buf(),
            folders,
        }]);
    }
    let tool = head
        .strip_prefix("${account.")
        .and_then(|tool| tool.strip_suffix('}'))
        .filter(|tool| !tool.is_empty())
        .ok_or_else(|| format!("{dir:?} must start with ${{account.<tool>}} or ${{tmp}}"))?;
    let accounts = account_dirs(tool);
    if !accounts.is_empty() {
        return Ok(accounts
            .into_iter()
            .map(|base| Target {
                base,
                folders: folders.clone(),
            })
            .collect());
    }
    let (Some(default_dir), Some(home)) = (default_dir, home) else {
        return Ok(Vec::new());
    };
    let rest = default_dir
        .strip_prefix("~/")
        .ok_or_else(|| format!("default_dir {default_dir:?} must start with ~/"))?;
    Ok(vec![Target {
        base: home.to_path_buf(),
        folders: folder_names(rest)?,
    }])
}

fn folder_names(rest: &str) -> Result<Vec<String>, String> {
    rest.split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .map(|part| {
            if is_plain_component(part) {
                Ok(part.to_owned())
            } else {
                Err(format!("{part:?} is not a plain folder name"))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn vars(windows: bool) -> Vars {
        Vars {
            port: 51234,
            token: "t\"k".into(),
            pid: 42,
            worktrees: vec!["/a".into(), "/b c".into()],
            uid: (!windows).then_some(501),
            tmp: "/tmp".into(),
            windows,
        }
    }

    #[test]
    fn a_whole_placeholder_keeps_its_type_and_text_is_substituted() {
        // Claude Code's lock file and Gemini CLI's discovery file (design §B.8).
        let claude = json!({ "pid": "${pid}", "workspaceFolders": "${worktrees}",
            "ideName": "Fyx", "transport": "ws", "authToken": "${token}" });
        assert_eq!(
            fill(&claude, &vars(false)).unwrap(),
            json!({ "pid": 42, "workspaceFolders": ["/a", "/b c"], "ideName": "Fyx",
                "transport": "ws", "authToken": "t\"k" })
        );
        let gemini = json!({ "port": "${port}", "workspacePath": "${worktrees.joined}",
            "authToken": "${token}", "ideInfo": { "name": "fyx", "displayName": "Fyx" },
            "windows": "${platform.windows}", "url": "ws://127.0.0.1:${port}/${worktrees}" });
        assert_eq!(
            fill(&gemini, &vars(true)).unwrap(),
            json!({ "port": 51234, "workspacePath": "/a;/b c", "authToken": "t\"k",
                "ideInfo": { "name": "fyx", "displayName": "Fyx" }, "windows": true,
                "url": "ws://127.0.0.1:51234//a;/b c" })
        );
        assert_eq!(
            fill(&json!(["${uid}", 7, null]), &vars(false)).unwrap(),
            json!([501, 7, null])
        );
        assert!(fill(&json!("${uid}"), &vars(true)).is_err());
        assert!(fill(&json!("${home}"), &vars(false)).is_err());
        assert!(fill(&json!("x ${port"), &vars(false)).is_err());
    }

    #[test]
    fn a_file_name_is_one_plain_lock_or_json_name() {
        let vars = vars(false);
        assert_eq!(file_name("${port}.lock", &vars).unwrap(), "51234.lock");
        assert_eq!(
            file_name("gemini-ide-server-${pid}-${port}.json", &vars).unwrap(),
            "gemini-ide-server-42-51234.json"
        );
        for bad in [
            "settings.json",
            "${port}.toml",
            "../${port}.lock",
            "a/${port}.lock",
            "${token}-${port}.lock",
            ".${port}.lock",
        ] {
            assert!(file_name(bad, &vars).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_dir_goes_to_each_account_or_the_default_or_tmp() {
        let home = Path::new("/home/u");
        let tmp = Path::new("/tmp");
        let accounts = |tool: &str| match tool {
            "claude" => vec![
                PathBuf::from("/w/claude-work"),
                PathBuf::from("/w/claude-me"),
            ],
            _ => Vec::new(),
        };
        assert_eq!(
            targets(
                "${account.claude}/ide",
                Some("~/.claude/ide"),
                accounts,
                Some(home),
                tmp
            )
            .unwrap(),
            vec![
                Target {
                    base: "/w/claude-work".into(),
                    folders: vec!["ide".into()]
                },
                Target {
                    base: "/w/claude-me".into(),
                    folders: vec!["ide".into()]
                },
            ]
        );
        assert_eq!(
            targets(
                "${account.codex}/ide",
                Some("~/.codex/ide"),
                accounts,
                Some(home),
                tmp
            )
            .unwrap(),
            vec![Target {
                base: home.into(),
                folders: vec![".codex".into(), "ide".into()]
            }]
        );
        assert_eq!(
            targets("${account.codex}/ide", None, accounts, Some(home), tmp).unwrap(),
            vec![]
        );
        assert_eq!(
            targets("${tmp}/gemini/ide", None, accounts, Some(home), tmp).unwrap(),
            vec![Target {
                base: tmp.into(),
                folders: vec!["gemini".into(), "ide".into()]
            }]
        );
        for bad in [
            "/etc",
            "~/.claude",
            "${tmp}/../etc",
            "${tmp}/gemini-${uid}",
            "${account.}/x",
            "${tmp}/a\\..\\b",
        ] {
            assert!(
                targets(bad, None, accounts, Some(home), tmp).is_err(),
                "{bad}"
            );
        }
        assert!(targets("${account.codex}", Some("/etc"), accounts, Some(home), tmp).is_err());
    }
}
