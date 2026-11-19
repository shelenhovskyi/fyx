//! Parsing of the OSC payloads the tap collects, on the IO thread.
//!
//! Everything here is untrusted program output. Text that may be shown outside the grid is checked or
//! cleaned: control characters, bidi overrides and invisible formatting characters never get through,
//! and every field has a size cap.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

/// Longest notification title or body, in characters, after cleaning.
pub const MAX_NOTIFY_TEXT: usize = 2048;
/// Longest cwd URI or path, command line, or user variable value.
pub const MAX_TEXT: usize = 4096;
/// Longest user variable name.
pub const MAX_USER_VAR_NAME: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OscEvent {
    /// OSC 7 (`file://host/path`), ConEmu's OSC 9;9 and VS Code's OSC 633;P;Cwd=.
    Cwd { source: Protocol, path: String },
    /// OSC 9 (iTerm2), OSC 777;notify (urxvt, Ghostty) and OSC 99 (kitty).
    Notify {
        source: Protocol,
        title: Option<String>,
        body: String,
    },
    /// ConEmu's OSC 9;4.
    Progress(Progress),
    /// OSC 133 (FinalTerm) and OSC 633 (VS Code) prompt and command marks.
    Shell { source: Protocol, mark: ShellMark },
    /// OSC 633;E: the command line, with the nonce the shell integration script was given.
    CommandLine { text: String, nonce: Option<String> },
    /// OSC 1337;SetUserVar (iTerm2, WezTerm).
    UserVar { name: String, value: String },
    /// OSC 7501 Program Status report.
    ProgramStatus(ProgramStatus),
}

impl OscEvent {
    /// Events that describe current state rather than something that happened: a newer one of the
    /// same key replaces an older one still queued.
    pub fn coalesce_key(&self) -> Option<CoalesceKey<'_>> {
        match self {
            Self::Cwd { .. } => Some(CoalesceKey::Cwd),
            Self::Progress(_) => Some(CoalesceKey::Progress),
            Self::UserVar { name, .. } => Some(CoalesceKey::UserVar(name)),
            Self::ProgramStatus(status) => Some(CoalesceKey::ProgramStatus(status.id.as_deref())),
            Self::Notify { .. } | Self::Shell { .. } | Self::CommandLine { .. } => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoalesceKey<'a> {
    Cwd,
    Progress,
    UserVar(&'a str),
    ProgramStatus(Option<&'a str>),
}

/// Which protocol an event came through, so the UI can always say where a status came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Protocol {
    Osc7,
    Osc9,
    Osc99,
    Osc133,
    Osc633,
    Osc777,
    Osc1337,
    Osc7501,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    Remove,
    Normal(u8),
    Error(Option<u8>),
    Indeterminate,
    Paused(Option<u8>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellMark {
    PromptStart,
    CommandStart,
    CommandExecuted,
    CommandFinished { exit_code: Option<i32> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StatusState {
    Idle,
    Working,
    Done,
    Blocked,
    Error,
    /// Not a state: removes the record and everything beneath it.
    Clear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BlockedKind {
    Permission,
    Question,
    Auth,
}

/// One OSC 7501 report (spec rev 0.4). A report replaces its record completely.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgramStatus {
    pub state: StatusState,
    /// `/`-separated record path; `None` is the root record.
    pub id: Option<String>,
    pub kind: Option<BlockedKind>,
    pub progress: Option<u8>,
    pub app: Option<String>,
    pub title: Option<String>,
    pub msg: Option<String>,
}

/// Parses one OSC payload (without `ESC ]` and the terminator).
pub fn parse(payload: &[u8]) -> Option<OscEvent> {
    let payload = std::str::from_utf8(payload).ok()?;
    let (code, rest) = payload.split_once(';').unwrap_or((payload, ""));
    match code {
        "7" => parse_osc7(rest),
        "9" => parse_osc9(rest),
        "99" => parse_osc99(rest),
        "133" => parse_shell_mark(Protocol::Osc133, rest),
        "633" => parse_osc633(rest),
        "777" => parse_osc777(rest),
        "1337" => parse_osc1337(rest),
        "7501" => parse_program_status(rest).map(OscEvent::ProgramStatus),
        _ => None,
    }
}

fn parse_osc7(uri: &str) -> Option<OscEvent> {
    let after_scheme = uri.strip_prefix("file://")?;
    // `file://host/path`: the host is not checked, a remote host's path is still informative.
    let path = &after_scheme[after_scheme.find('/')?..];
    let path = percent_encoding::percent_decode_str(path)
        .decode_utf8()
        .ok()?;
    // `/C:/Users/...` on Windows.
    let path = match path.as_bytes() {
        [b'/', drive, b':', ..] if drive.is_ascii_alphabetic() => &path[1..],
        _ => &path,
    };
    cwd(Protocol::Osc7, path)
}

fn cwd(source: Protocol, path: &str) -> Option<OscEvent> {
    if path.is_empty() || path.len() > MAX_TEXT || path.chars().any(char::is_control) {
        return None;
    }
    Some(OscEvent::Cwd {
        source,
        path: path.to_owned(),
    })
}

fn parse_osc9(rest: &str) -> Option<OscEvent> {
    // ConEmu's `9;<n>;...` subcommands; anything else is an iTerm2 notification.
    if let Some((sub, args)) = rest.split_once(';')
        && !sub.is_empty()
        && sub.bytes().all(|byte| byte.is_ascii_digit())
    {
        return match sub {
            "4" => parse_progress(args).map(OscEvent::Progress),
            "9" => cwd(Protocol::Osc9, args.trim_matches('"')),
            _ => None,
        };
    }
    if rest == "4" {
        return Some(OscEvent::Progress(Progress::Remove));
    }
    notify(Protocol::Osc9, None, rest)
}

fn parse_progress(args: &str) -> Option<Progress> {
    let mut parts = args.split(';');
    let state = parts.next().unwrap_or("0");
    let value = parts
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .map(|value| value.min(100) as u8);
    Some(match state {
        "0" => Progress::Remove,
        "1" => Progress::Normal(value.unwrap_or(0)),
        "2" => Progress::Error(value),
        "3" => Progress::Indeterminate,
        "4" => Progress::Paused(value),
        _ => return None,
    })
}

fn notify(source: Protocol, title: Option<&str>, body: &str) -> Option<OscEvent> {
    let title = title.map(clean_text).filter(|title| !title.is_empty());
    let body = clean_text(body);
    if body.is_empty() && title.is_none() {
        return None;
    }
    Some(OscEvent::Notify {
        source,
        title,
        body,
    })
}

/// Drops control, bidi-override and invisible formatting characters, collapses whitespace runs, and
/// caps the length.
pub fn clean_text(text: &str) -> String {
    let mut cleaned = String::with_capacity(text.len().min(MAX_NOTIFY_TEXT));
    let mut last_space = true;
    for c in text.chars() {
        if cleaned.chars().count() >= MAX_NOTIFY_TEXT {
            break;
        }
        if c.is_whitespace() || c.is_control() {
            if !last_space {
                cleaned.push(' ');
                last_space = true;
            }
        } else if !is_invisible_format(c) {
            cleaned.push(c);
            last_space = false;
        }
    }
    cleaned.truncate(cleaned.trim_end().len());
    cleaned
}

fn is_invisible_format(c: char) -> bool {
    matches!(c,
        '\u{00AD}' | '\u{061C}' | '\u{180E}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}'
        | '\u{2060}'..='\u{206F}' | '\u{FEFF}' | '\u{FFF9}'..='\u{FFFB}')
}

fn parse_osc99(rest: &str) -> Option<OscEvent> {
    // kitty: `99;metadata;payload`, metadata `key=value:key=value`. Only single-chunk notifications
    // (`d=1`, the default) are shown; `p=title` sets the title, the body is `p=body` (the default).
    let (metadata, payload) = rest.split_once(';')?;
    let mut done = true;
    let mut part = "body";
    let mut base64 = false;
    for pair in metadata.split(':').filter(|pair| !pair.is_empty()) {
        match pair.split_once('=') {
            Some(("d", value)) => done = value != "0",
            Some(("p", value)) => part = value,
            Some(("e", value)) => base64 = value == "1",
            _ => {}
        }
    }
    if !done {
        return None;
    }
    let text = if base64 {
        String::from_utf8(BASE64.decode(payload).ok()?).ok()?
    } else {
        payload.to_owned()
    };
    match part {
        "title" => notify(Protocol::Osc99, Some(&text), ""),
        "body" => notify(Protocol::Osc99, None, &text),
        _ => None,
    }
}

fn parse_shell_mark(source: Protocol, rest: &str) -> Option<OscEvent> {
    let mut parts = rest.split(';');
    let mark = match parts.next()? {
        "A" => ShellMark::PromptStart,
        "B" => ShellMark::CommandStart,
        "C" => ShellMark::CommandExecuted,
        "D" => ShellMark::CommandFinished {
            exit_code: parts.next().and_then(|code| code.parse().ok()),
        },
        _ => return None,
    };
    Some(OscEvent::Shell { source, mark })
}

fn parse_osc633(rest: &str) -> Option<OscEvent> {
    let (sub, args) = rest.split_once(';').unwrap_or((rest, ""));
    match sub {
        "A" | "B" | "C" | "D" => parse_shell_mark(Protocol::Osc633, rest),
        "E" => {
            let (line, nonce) = match args.split_once(';') {
                Some((line, nonce)) => (line, Some(nonce)),
                None => (args, None),
            };
            let text = unescape_633(line)?;
            if text.len() > MAX_TEXT {
                return None;
            }
            Some(OscEvent::CommandLine {
                text,
                nonce: nonce.map(str::to_owned),
            })
        }
        "P" => {
            let (key, value) = args.split_once('=')?;
            match key {
                "Cwd" => cwd(Protocol::Osc633, &unescape_633(value)?),
                _ => None,
            }
        }
        _ => None,
    }
}

/// VS Code escapes `\` as `\\` and bytes below 0x20 and `;` as `\xAB`.
fn unescape_633(text: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut input = text.bytes();
    while let Some(byte) = input.next() {
        if byte != b'\\' {
            bytes.push(byte);
            continue;
        }
        match input.next()? {
            b'\\' => bytes.push(b'\\'),
            b'x' => {
                let high = (input.next()? as char).to_digit(16)?;
                let low = (input.next()? as char).to_digit(16)?;
                bytes.push((high * 16 + low) as u8);
            }
            _ => return None,
        }
    }
    String::from_utf8(bytes).ok()
}

fn parse_osc777(rest: &str) -> Option<OscEvent> {
    let mut parts = rest.splitn(3, ';');
    if parts.next()? != "notify" {
        return None;
    }
    let title = parts.next().unwrap_or("");
    let body = parts.next().unwrap_or("");
    notify(Protocol::Osc777, Some(title), body)
}

fn parse_osc1337(rest: &str) -> Option<OscEvent> {
    let (name, value) = rest.strip_prefix("SetUserVar=")?.split_once('=')?;
    if name.is_empty()
        || name.len() > MAX_USER_VAR_NAME
        || !name.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'.' || byte == b'-'
        })
    {
        return None;
    }
    let value = String::from_utf8(BASE64.decode(value).ok()?).ok()?;
    if value.len() > MAX_TEXT || value.chars().any(char::is_control) {
        return None;
    }
    Some(OscEvent::UserVar {
        name: name.to_owned(),
        value,
    })
}

/// OSC 7501 limits (spec rev 0.4).
pub mod program_status_limits {
    pub const MAX_SEQUENCE: usize = 4096;
    pub const MAX_KEY: usize = 16;
    pub const MAX_ID: usize = 128;
    pub const MAX_ID_SEGMENT: usize = 32;
    pub const MAX_ID_DEPTH: usize = 8;
    pub const MAX_APP: usize = 32;
    pub const MAX_TITLE_ENCODED: usize = 256;
    pub const MAX_TITLE: usize = 192;
    pub const MAX_MSG_ENCODED: usize = 2732;
    pub const MAX_MSG: usize = 2048;
    /// Records a terminal keeps; the least recently updated one goes first.
    pub const MAX_RECORDS: usize = 64;
}

fn parse_program_status(rest: &str) -> Option<ProgramStatus> {
    use program_status_limits::*;

    // `ESC ] 7501 ;` plus the `ESC \` terminator.
    if rest.len() + 8 > MAX_SEQUENCE || rest.starts_with('?') {
        return None;
    }
    let mut state = None;
    let mut id = None;
    let mut kind = None;
    let mut progress = None;
    let mut app = None;
    let mut title = None;
    let mut msg = None;
    for pair in rest.split(':') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        if key.len() > MAX_KEY {
            continue;
        }
        match key {
            "state" => {
                state = Some(match value {
                    "idle" => StatusState::Idle,
                    "working" => StatusState::Working,
                    "done" => StatusState::Done,
                    "blocked" => StatusState::Blocked,
                    "error" => StatusState::Error,
                    "clear" => StatusState::Clear,
                    // An unknown state makes the whole report ignored.
                    _ => return None,
                })
            }
            "id" => id = Some(value),
            "kind" => {
                kind = match value {
                    "permission" => Some(BlockedKind::Permission),
                    "question" => Some(BlockedKind::Question),
                    "auth" => Some(BlockedKind::Auth),
                    _ => None,
                }
            }
            "progress" => progress = value.parse::<u8>().ok().filter(|value| *value <= 100),
            "app" => {
                app = (!value.is_empty()
                    && value.len() <= MAX_APP
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"_.+-".contains(&byte)))
                .then_some(value)
            }
            "title" => title = Some(decode_status_text(value, MAX_TITLE_ENCODED, MAX_TITLE)?),
            "msg" => msg = Some(decode_status_text(value, MAX_MSG_ENCODED, MAX_MSG)?),
            _ => {}
        }
    }
    let state = state?;
    let id = match id {
        Some(id) => Some(valid_record_id(id)?.to_owned()),
        None => None,
    };
    Some(ProgramStatus {
        state,
        id,
        kind: kind.filter(|_| state == StatusState::Blocked),
        progress: progress.filter(|_| matches!(state, StatusState::Working | StatusState::Blocked)),
        app: app.map(str::to_owned),
        title,
        msg,
    })
}

fn valid_record_id(id: &str) -> Option<&str> {
    use program_status_limits::*;
    let segments: Vec<&str> = id.split('/').collect();
    let valid = id.len() <= MAX_ID
        && segments.len() <= MAX_ID_DEPTH
        && segments.iter().all(|segment| {
            !segment.is_empty()
                && segment.len() <= MAX_ID_SEGMENT
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_graphic() && byte != b':' && byte != b';')
        });
    valid.then_some(id)
}

/// Decodes a base64 UTF-8 field; `None` (the whole report is dropped) if it is too long, not base64,
/// or contains a control character. Invisible formatting characters are removed.
fn decode_status_text(value: &str, max_encoded: usize, max_decoded: usize) -> Option<String> {
    if value.len() > max_encoded {
        return None;
    }
    let text = String::from_utf8(BASE64.decode(value).ok()?).ok()?;
    if text.len() > max_decoded || text.chars().any(char::is_control) {
        return None;
    }
    Some(text.chars().filter(|c| !is_invisible_format(*c)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_str(payload: &str) -> Option<OscEvent> {
        parse(payload.as_bytes())
    }

    fn b64(text: &str) -> String {
        BASE64.encode(text)
    }

    #[test]
    fn cwd_from_osc7_9_9_and_633() {
        assert_eq!(
            parse_str("7;file://host/home/me/a%20b"),
            Some(OscEvent::Cwd {
                source: Protocol::Osc7,
                path: "/home/me/a b".into()
            })
        );
        assert_eq!(
            parse_str("7;file://pc/C:/Users/me"),
            Some(OscEvent::Cwd {
                source: Protocol::Osc7,
                path: "C:/Users/me".into()
            })
        );
        assert_eq!(
            parse_str("9;9;\"C:\\work\""),
            Some(OscEvent::Cwd {
                source: Protocol::Osc9,
                path: "C:\\work".into()
            })
        );
        assert_eq!(
            parse_str("633;P;Cwd=/srv/x\\x3by"),
            Some(OscEvent::Cwd {
                source: Protocol::Osc633,
                path: "/srv/x;y".into()
            })
        );
        assert_eq!(parse_str("7;http://host/x"), None);
        assert_eq!(parse_str("7;file://host/a%0Ab"), None);
    }

    #[test]
    fn notifications_are_cleaned() {
        assert_eq!(
            parse_str("9;Build \u{202E}done\u{200B}\n  now"),
            Some(OscEvent::Notify {
                source: Protocol::Osc9,
                title: None,
                body: "Build done now".into()
            })
        );
        assert_eq!(
            parse_str("777;notify;Claude;Needs input"),
            Some(OscEvent::Notify {
                source: Protocol::Osc777,
                title: Some("Claude".into()),
                body: "Needs input".into()
            })
        );
        assert_eq!(
            parse_str(&format!("99;e=1;{}", b64("hello"))),
            Some(OscEvent::Notify {
                source: Protocol::Osc99,
                title: None,
                body: "hello".into()
            })
        );
        assert_eq!(parse_str("99;i=1:d=0;partial"), None);
        assert_eq!(parse_str("777;preexec"), None);
        let long = "x".repeat(MAX_NOTIFY_TEXT + 10);
        let Some(OscEvent::Notify { body, .. }) = parse_str(&format!("9;{long}")) else {
            panic!()
        };
        assert_eq!(body.len(), MAX_NOTIFY_TEXT);
    }

    #[test]
    fn conemu_progress() {
        assert_eq!(
            parse_str("9;4;1;42"),
            Some(OscEvent::Progress(Progress::Normal(42)))
        );
        assert_eq!(
            parse_str("9;4;2"),
            Some(OscEvent::Progress(Progress::Error(None)))
        );
        assert_eq!(
            parse_str("9;4;3;0"),
            Some(OscEvent::Progress(Progress::Indeterminate))
        );
        assert_eq!(
            parse_str("9;4;0;0"),
            Some(OscEvent::Progress(Progress::Remove))
        );
        assert_eq!(
            parse_str("9;4;1;250"),
            Some(OscEvent::Progress(Progress::Normal(100)))
        );
        assert_eq!(parse_str("9;2;message box"), None);
    }

    #[test]
    fn shell_marks_and_command_lines() {
        assert_eq!(
            parse_str("133;D;1"),
            Some(OscEvent::Shell {
                source: Protocol::Osc133,
                mark: ShellMark::CommandFinished { exit_code: Some(1) }
            })
        );
        assert_eq!(
            parse_str("133;A;cl=m"),
            Some(OscEvent::Shell {
                source: Protocol::Osc133,
                mark: ShellMark::PromptStart
            })
        );
        assert_eq!(
            parse_str("633;C"),
            Some(OscEvent::Shell {
                source: Protocol::Osc633,
                mark: ShellMark::CommandExecuted
            })
        );
        assert_eq!(
            parse_str("633;E;echo a\\x3b b \\\\;n0nce"),
            Some(OscEvent::CommandLine {
                text: "echo a; b \\".into(),
                nonce: Some("n0nce".into())
            })
        );
        assert_eq!(parse_str("633;E;bad\\q"), None);
    }

    #[test]
    fn user_vars() {
        assert_eq!(
            parse_str(&format!("1337;SetUserVar=agent_state={}", b64("working"))),
            Some(OscEvent::UserVar {
                name: "agent_state".into(),
                value: "working".into()
            })
        );
        assert_eq!(
            parse_str(&format!("1337;SetUserVar=a b={}", b64("x"))),
            None
        );
        assert_eq!(
            parse_str(&format!("1337;SetUserVar=v={}", b64("a\x1bb"))),
            None
        );
        assert_eq!(parse_str("1337;File=inline=1:AAAA"), None);
    }

    #[test]
    fn program_status_reports() {
        assert_eq!(
            parse_str("7501;state=working:app=claude-code:progress=40:kind=auth"),
            Some(OscEvent::ProgramStatus(ProgramStatus {
                state: StatusState::Working,
                id: None,
                kind: None,
                progress: Some(40),
                app: Some("claude-code".into()),
                title: None,
                msg: None,
            }))
        );
        let report = format!(
            "7501;state=blocked:kind=permission:id=build/test:title={}:msg={}:progress=7",
            b64("Run tests?"),
            b64("cargo \u{202E}test")
        );
        assert_eq!(
            parse_str(&report),
            Some(OscEvent::ProgramStatus(ProgramStatus {
                state: StatusState::Blocked,
                id: Some("build/test".into()),
                kind: Some(BlockedKind::Permission),
                progress: Some(7),
                app: None,
                title: Some("Run tests?".into()),
                msg: Some("cargo test".into()),
            }))
        );
        // A repeated key takes its last value; a malformed pair is skipped alone.
        let Some(OscEvent::ProgramStatus(status)) =
            parse_str("7501;state=idle:state=done:app=bad app:junk")
        else {
            panic!()
        };
        assert_eq!((status.state, status.app), (StatusState::Done, None));
    }

    #[test]
    fn invalid_program_status_reports_are_dropped() {
        assert_eq!(parse_str("7501;state=sleeping"), None);
        assert_eq!(parse_str("7501;app=x"), None);
        assert_eq!(parse_str("7501;?"), None);
        assert_eq!(parse_str("7501;state=idle:id=a//b"), None);
        assert_eq!(
            parse_str(&format!("7501;state=idle:id={}", "a/".repeat(9) + "a")),
            None
        );
        assert_eq!(
            parse_str(&format!("7501;state=done:msg={}", b64("line\nbreak"))),
            None
        );
        assert_eq!(parse_str("7501;state=done:title=!!notbase64"), None);
        assert_eq!(
            parse_str(&format!("7501;state=done:title={}", b64(&"t".repeat(193)))),
            None
        );
        assert_eq!(
            parse_str(&format!("7501;state=idle:x={}", "y".repeat(4096))),
            None
        );
    }
}
