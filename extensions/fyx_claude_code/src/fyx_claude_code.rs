//! `fyx_claude_code` (plan 5.15, `docs/fyx/wit-0.4/design.md` "The first extensions"): Claude
//! Code in Fyx's terminal, connected to the editor.
//!
//! - **Profile.** "Claude Code" in the terminal panel's "+" menu runs `claude --ide` in the
//!   active worktree, with an account profile the user picks (`account_tool = "claude"`).
//! - **Env** (profile terminals only): `CLAUDE_CODE_SSE_PORT` (the `ide` listener's port),
//!   `ENABLE_IDE_INTEGRATION=true` and `CLAUDE_CODE_PLUGIN_DIRS=${package}/claude-plugin`.
//! - **IDE protocol** ([`ide`]): the host's `mcp-websocket` listener `ide` and the lock file
//!   `${account.claude}/ide/${port}.lock`; the extension offers `openFile`, `openDiff` (a
//!   blocking proposal in agent review), `close_tab`, `closeAllDiffTabs` and `getDiagnostics`,
//!   and sends `selection_changed` and `at_mentioned`.
//! - **Hooks** ([`hooks`]): the plugin's command hooks reach the extension through `fyx-relay`;
//!   `UserPromptSubmit` begins an agent turn, `Stop` ends it with the files it wrote, and the
//!   tool and waiting message go on the terminal's status.
//! - **Compat test** (`compat`): recorded protocol transcripts of a pinned Claude Code version,
//!   replayed against [`ide`] and [`hooks`].
//!
//! The protocol core is plain Rust, tested natively. Built for 0.4, the extension registers
//! [`ide::tools`] on the `ide` listener and answers `io-event::mcp-call` through
//! [`ide::Server`] ([`wired`]). Until the SDK has `context` (plan 5.14), no absolute path maps
//! to a worktree, so `openFile`, `openDiff` and a one-file `getDiagnostics` are refused as
//! outside every worktree; `openDiff` becomes a proposal with `review`, and the hooks arrive
//! with the relay. The staged 0.4 WIT has no diagnostics source, so `getDiagnostics` answers
//! an empty list.

pub mod hooks;
pub mod ide;
#[cfg(feature = "fyx-ui-0-4")]
mod wired;

#[cfg(test)]
mod compat;

use ui_ext_sdk::*;

const PANEL: &str = "claude-code";

/// What the panel says the extension connects, in order.
pub const FEATURES: [&str; 4] = [
    "Opens files and reads diagnostics for Claude.",
    "Shows Claude's edits hunk by hunk in agent review before Claude writes them.",
    "Sends your selection, and files you mention, to Claude.",
    "Marks the terminal tab with Claude's tool, its waits and its turns.",
];

pub struct ClaudeCode {
    sessions: hooks::Sessions,
    #[cfg(feature = "fyx-ui-0-4")]
    ide: ide::Server,
}

impl Extension for ClaudeCode {
    fn new() -> Self {
        // Registered before the listener opens; the host offers them when it does.
        #[cfg(feature = "fyx-ui-0-4")]
        wired::register_tools();
        Self {
            sessions: hooks::Sessions::default(),
            #[cfg(feature = "fyx-ui-0-4")]
            ide: ide::Server::default(),
        }
    }

    #[cfg(feature = "fyx-ui-0-4")]
    fn handle_io(&mut self, event: IoEvent) -> Result<(), String> {
        match event {
            IoEvent::McpCall(call) if call.listener == ide::LISTENER => {
                self.ide.call(
                    &mut wired::SdkHost,
                    call.call_id,
                    &call.tool,
                    &call.arguments,
                );
            }
            IoEvent::McpCancelled(call) => self.ide.cancelled(call),
            _ => {}
        }
        Ok(())
    }

    fn surfaces(&self) -> Vec<Surface> {
        vec![Surface::panel(
            PANEL,
            "Claude Code",
            DockPosition::Right,
            Icon::Sparkle,
            320,
        )]
    }

    fn mount(&mut self, surface_id: &str) -> Result<Vec<Node>, String> {
        if surface_id != PANEL {
            return Err(format!("unknown surface {surface_id:?}"));
        }
        let mut ids = IdAllocator::new();
        let mut tree = TreeBuilder::new(&mut ids);
        tree.flex(FlexProps::column().gap(8).padding(8), |tree| {
            tree.text(TextProps::new("Claude Code").bold());
            tree.text(
                TextProps::new(
                    "Start Claude Code from the terminal panel's + menu. Pick the account it \
                     runs with; its terminal is yours, and this extension cannot read it.",
                )
                .color(ColorToken::TextMuted),
            );
            for feature in FEATURES {
                tree.text(TextProps::new(feature));
            }
            let sessions = match self.sessions.len() {
                0 => "No Claude session is running.".to_owned(),
                1 => "1 Claude session is running.".to_owned(),
                count => format!("{count} Claude sessions are running."),
            };
            tree.text(TextProps::new(sessions).color(ColorToken::TextMuted));
        });
        Ok(tree.finish())
    }
}

#[cfg(not(any(feature = "fyx-ui-0-2", feature = "fyx-ui-0-3")))]
register_extension!(ClaudeCode, v0_1);
#[cfg(all(feature = "fyx-ui-0-2", not(feature = "fyx-ui-0-3")))]
register_extension!(ClaudeCode, v0_2);
#[cfg(feature = "fyx-ui-0-3")]
register_extension!(ClaudeCode);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_says_what_it_connects() {
        let mut extension = ClaudeCode::new();
        let nodes = extension.mount(PANEL).unwrap();
        let texts: Vec<String> = nodes
            .iter()
            .filter_map(|node| match &node.props {
                Props::Text(text) => Some(text.content.clone()),
                _ => None,
            })
            .collect();
        for feature in FEATURES {
            assert!(texts.iter().any(|text| text == feature), "{texts:?}");
        }
        assert!(
            texts
                .iter()
                .any(|text| text == "No Claude session is running.")
        );
        assert!(extension.mount("other").is_err());
    }
}
