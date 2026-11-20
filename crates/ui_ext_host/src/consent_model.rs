//! The 0.4 consent model (docs/fyx/wit-0.4/design.md §C.1–§C.5) as the consent UI uses it:
//! W2c's types from `ui_ext_wasm::permissions` and `ui_ext_wasm::journal`, and how the host
//! words a diff and a journal entry. The broker is `crate::consent::ConsentStore` (`install`,
//! `set_item`, `item_state`, `diff`); the journal file is `crate::journal::JournalStore`.

pub use ui_ext_wasm::journal::{JournalEntry, JournalKind, JournalPage};
pub use ui_ext_wasm::permissions::{
    ConsentDiff, ConsentItem, ItemLine, Tier as ConsentTier, Widened, consent_diff_lines,
    consent_items,
};

/// What the dialog and the page draw per line.
pub trait ItemLineExt {
    /// Whether the user ticks it (T2); T0, T1 and T3 lines have no checkbox.
    fn has_checkbox(&self) -> bool;
}

impl ItemLineExt for ItemLine {
    fn has_checkbox(&self) -> bool {
        self.tier == ConsentTier::T2
    }
}

/// The lines an update adds or widens, in that order (§C.5).
pub fn new_lines(diff: &ConsentDiff) -> Vec<ItemLine> {
    diff.added
        .iter()
        .cloned()
        .chain(diff.widened.iter().map(|widened| widened.line.clone()))
        .collect()
}

/// One sentence for a held update's card: "New in this version: Runs claude; Connects to the
/// internet (evil.example.com). Removed: Shows approval cards."
pub fn diff_text(diff: &ConsentDiff) -> String {
    let describe = |line: &ItemLine, details: &[String]| {
        if details.is_empty() {
            line.title.clone()
        } else {
            format!("{} ({})", line.title, details.join(", "))
        }
    };
    let mut parts = Vec::new();
    let new: Vec<String> = diff
        .added
        .iter()
        .map(|line| describe(line, &line.details))
        .chain(
            diff.widened
                .iter()
                .map(|widened| describe(&widened.line, &widened.new_details)),
        )
        .collect();
    if !new.is_empty() {
        parts.push(format!("New in this version: {}.", new.join("; ")));
    }
    if !diff.removed.is_empty() {
        let removed: Vec<String> = diff.removed.iter().map(|line| line.title.clone()).collect();
        parts.push(format!("Removed: {}.", removed.join("; ")));
    }
    parts.join(" ")
}

/// Plan 7.12: a line's title or detail as people read it. The stored text stays as it is (an
/// update is compared detail by detail), only what is drawn changes: `${package}` and the other
/// templates become words, listener protocols their names, and a hook channel loses its exit code
/// when that is 0.
pub fn readable_text(text: &str) -> String {
    match text {
        "mcpwebsocket" => return "MCP over WebSocket".into(),
        "mcpstreamablehttp" => return "MCP over HTTP".into(),
        "websocket" => return "WebSocket".into(),
        "http" => return "HTTP".into(),
        _ => {}
    }
    if let Some(channel) = text.strip_prefix("channel ") {
        let (name, exit) = match channel.split_once(" (on failure: exit ") {
            Some((name, exit)) => (name, exit.trim_end_matches(')')),
            None => (channel, "0"),
        };
        return if exit == "0" {
            format!("{name} hook")
        } else {
            format!("{name} hook (exits {exit} when Fyx does not answer)")
        };
    }
    let mut readable = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        let Some(end) = rest[start..].find('}') else {
            break;
        };
        readable.push_str(&rest[..start]);
        let placeholder = &rest[start + 2..start + end];
        let words = match placeholder.split('.').collect::<Vec<_>>().as_slice() {
            ["package"] => "<this extension's folder>".to_string(),
            ["relay"] => "<Fyx's hook relay>".to_string(),
            ["pid"] => "<Fyx's process id>".to_string(),
            ["port"] => "<port>".to_string(),
            ["listen", name, "port"] => format!("<the port Fyx picks for {name}>"),
            ["listen", name, "token"] => format!("<{name}'s token>"),
            ["account", tool] => format!("<the {tool} account you choose>"),
            _ => format!("${{{placeholder}}}"),
        };
        readable.push_str(&words);
        rest = &rest[start + end + 1..];
    }
    readable.push_str(rest);
    readable
}

/// Every warning once, in order of the lines: the dialog lists the risks together, once.
pub fn unique_warnings(lines: &[ItemLine]) -> Vec<String> {
    let mut warnings: Vec<String> = Vec::new();
    for warning in lines.iter().flat_map(|line| &line.warnings) {
        if !warnings.contains(warning) {
            warnings.push(warning.clone());
        }
    }
    warnings
}

/// When an entry happened. The journal is a file on disk: a time past what `SystemTime` holds (a
/// damaged or hand-edited line) reads as the epoch rather than panicking while the page renders.
pub fn entry_time(entry: &JournalEntry) -> std::time::SystemTime {
    std::time::UNIX_EPOCH
        .checked_add(std::time::Duration::from_millis(entry.at_ms))
        .unwrap_or(std::time::UNIX_EPOCH)
}

/// How the extension page words an entry.
pub fn journal_line(kind: &JournalKind) -> String {
    match kind {
        JournalKind::ProgramStarted {
            program, args, cwd, ..
        } => {
            let mut line = format!("Ran {program}");
            for arg in args {
                line.push(' ');
                line.push_str(arg);
            }
            if let Some(cwd) = cwd {
                line.push_str(&format!(" in {cwd}"));
            }
            line
        }
        JournalKind::ProgramExited { program, code } => match code {
            Some(code) => format!("{program} exited with {code}"),
            None => format!("{program} ended"),
        },
        JournalKind::TerminalRead {
            terminal,
            scope,
            bytes,
        } => format!("Read {scope} of the terminal \"{terminal}\" ({bytes} bytes)"),
        JournalKind::TerminalWrite {
            terminal,
            text,
            submit,
        } => format!(
            "Typed into the terminal \"{terminal}\": {text}{}",
            if *submit { " (and pressed Enter)" } else { "" }
        ),
        JournalKind::ListenerOpened { listener, port } => {
            format!("Listened on port {port} ({listener})")
        }
        JournalKind::Connections { listener, count } => {
            format!("{count} connections to {listener} in an hour")
        }
        JournalKind::DiscoveryFile { path, written } => format!(
            "{} the discovery file {path}",
            if *written { "Wrote" } else { "Removed" }
        ),
        JournalKind::Relay { channel, count } => {
            format!("{count} hook requests on {channel} in an hour")
        }
        JournalKind::ProposedEdit { path, outcome } => {
            format!("Proposed an edit to {path}: {outcome}")
        }
        JournalKind::Fetched { host, count } => match count {
            1 => format!("Fetched from {host}"),
            count => format!("Fetched from {host} {count} times"),
        },
        JournalKind::FileEdited { path, count } => match count {
            1 => format!("Edited {path}"),
            count => format!("Edited {path} {count} times"),
        },
        JournalKind::ConsentChanged { item_key, allowed } => format!(
            "{item_key} {}",
            if *allowed { "turned on" } else { "turned off" }
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry_at(at_ms: u64) -> JournalEntry {
        JournalEntry {
            at_ms,
            kind: JournalKind::TerminalRead {
                terminal: "t".into(),
                scope: "lines:20".into(),
                bytes: 10,
            },
        }
    }

    #[test]
    fn templates_protocols_and_hook_channels_read_as_words() {
        assert_eq!(
            readable_text("CLAUDE_CODE_PLUGIN_DIRS=${package}/plugin"),
            "CLAUDE_CODE_PLUGIN_DIRS=<this extension's folder>/plugin"
        );
        assert_eq!(
            readable_text("CLAUDE_CODE_SSE_PORT=${listen.ide.port}"),
            "CLAUDE_CODE_SSE_PORT=<the port Fyx picks for ide>"
        );
        assert_eq!(
            readable_text("${account.claude}/ide/${port}.lock"),
            "<the claude account you choose>/ide/<port>.lock"
        );
        assert_eq!(readable_text("mcpwebsocket"), "MCP over WebSocket");
        assert_eq!(
            readable_text("channel session-start (on failure: exit 0)"),
            "session-start hook"
        );
        assert_eq!(
            readable_text("channel pre-tool-use (on failure: exit 2)"),
            "pre-tool-use hook (exits 2 when Fyx does not answer)"
        );
        assert_eq!(readable_text("with -c"), "with -c");
        assert_eq!(
            readable_text("odd ${unknown} and ${open"),
            "odd ${unknown} and ${open"
        );
    }

    /// `u64::MAX` ms overflows Windows' `SystemTime` (100 ns ticks in a `u64`); Unix's holds it.
    #[test]
    fn an_entry_time_past_what_system_time_holds_does_not_panic() {
        let far = entry_time(&entry_at(u64::MAX));
        if cfg!(windows) {
            assert_eq!(far, std::time::UNIX_EPOCH);
        }
        assert_eq!(
            entry_time(&entry_at(1_000)),
            std::time::UNIX_EPOCH + std::time::Duration::from_secs(1)
        );
    }
}
