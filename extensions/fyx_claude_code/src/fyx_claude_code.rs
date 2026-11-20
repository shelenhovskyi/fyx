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
//! [`ide::Server`] ([`wired`]); Claude's paths map to worktree files through `context`, which
//! also feeds `selection_changed`, and `openDiff` is a proposal in agent review (`review`),
//! withdrawn when Claude closes its tab first. `getDiagnostics` reads `context.diagnostics`;
//! its answers are cached until `diagnostics-changed`. The hooks arrive as
//! `io-event::relay-request` and are answered at once ([`hooks::answer`]: exit 0, and a `deny`
//! for a `Bash` command zsh would silently change, [`zsh`]); their turns name
//! Claude's sessions, and so do the proposals of the one session in a turn. A rejected
//! `openDiff` ends that turn ([`hooks::Sessions::diff_rejected`]).

pub mod hooks;
pub mod ide;
pub mod zsh;
#[cfg(feature = "fyx-ui-0-4")]
mod wired;

#[cfg(feature = "fyx-ui-0-4")]
use std::collections::BTreeMap;

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
    /// The last `selection_changed` params sent, so an unchanged selection is not sent again.
    #[cfg(feature = "fyx-ui-0-4")]
    selection: Option<String>,
    /// The terminal each Claude session's hooks last ran in, for its status.
    #[cfg(feature = "fyx-ui-0-4")]
    terminals: BTreeMap<String, terminals::TerminalHandle>,
}

#[cfg(feature = "fyx-ui-0-4")]
impl ClaudeCode {
    /// The host, with the session `openDiff` proposals are named by.
    fn host(&self) -> wired::SdkHost {
        wired::SdkHost::new(self.sessions.current())
    }

    /// One hook, through `fyx-relay`. A hook the extension cannot read is logged; every hook is
    /// answered at once ([`hooks::answer`]), so Claude never waits on Fyx.
    fn hook(&mut self, request: relay::RelayRequest) {
        let input = hooks::Channel::from_relay(&request.channel)
            .ok_or_else(|| format!("unknown relay channel {:?}", request.channel))
            .and_then(|channel| hooks::HookInput::parse(channel, &request.stdin));
        let (exit_code, stdout, stderr) = hooks::answer(input.as_ref().ok(), &request.argv);
        match input {
            Ok(input) => {
                // As in the compat replay: a new turn reads diagnostics afresh.
                if input.channel == hooks::Channel::UserPrompt {
                    self.ide.invalidate_diagnostics();
                }
                if let Some(terminal) = request.terminal {
                    self.terminals.insert(input.session.clone(), terminal);
                }
                let effects = self.sessions.apply(&input, wired::to_ref);
                let terminal = self.terminals.get(&input.session).copied();
                wired::run_hook_effects(terminal, &effects);
                if input.channel == hooks::Channel::SessionEnd {
                    self.terminals.remove(&input.session);
                }
            }
            Err(error) => log(
                LogLevel::Warn,
                &format!("a Claude hook was ignored: {error}"),
            ),
        }
        if let Err(error) = relay::respond(request.id, exit_code, &stdout, &stderr) {
            log(
                LogLevel::Warn,
                &format!(
                    "the answer to hook {} was refused: {}",
                    request.id, error.message
                ),
            );
        }
    }
}

impl Extension for ClaudeCode {
    fn new() -> Self {
        // Registered before the listener opens; the host offers them when it does.
        #[cfg(feature = "fyx-ui-0-4")]
        {
            wired::register_tools();
            wired::watch_context();
            wired::watch_diagnostics();
        }
        Self {
            sessions: hooks::Sessions::default(),
            #[cfg(feature = "fyx-ui-0-4")]
            ide: ide::Server::default(),
            #[cfg(feature = "fyx-ui-0-4")]
            selection: None,
            #[cfg(feature = "fyx-ui-0-4")]
            terminals: BTreeMap::new(),
        }
    }

    #[cfg(feature = "fyx-ui-0-4")]
    fn handle_io(&mut self, event: IoEvent) -> Result<(), String> {
        match event {
            IoEvent::McpCall(call) if call.listener == ide::LISTENER => {
                let mut host = self.host();
                self.ide
                    .call(&mut host, call.call_id, &call.tool, &call.arguments);
            }
            IoEvent::McpCancelled(call) => self.ide.cancelled(&mut self.host(), call),
            IoEvent::RelayRequest((id, channel, terminal, argv, stdin)) => {
                self.hook(relay::RelayRequest {
                    id,
                    channel,
                    terminal,
                    argv,
                    stdin,
                });
            }
            // A client that just finished the handshake gets the current selection.
            IoEvent::McpInitialized(session) if session.listener == ide::LISTENER => {
                if let Some(params) = &self.selection {
                    wired::notify_selection(Some(session.connection), params);
                }
            }
            IoEvent::DiagnosticsChanged(_) => self.ide.invalidate_diagnostics(),
            IoEvent::ReviewDecided((proposal, outcome)) => {
                let answer = wired::diff_answer(outcome);
                let answered = self.ide.review_decided(&mut self.host(), proposal, &answer);
                // Claude ends the turn after DIFF_REJECTED without a Stop hook.
                if answered && answer == ide::DiffAnswer::Rejected {
                    let effects = self.sessions.diff_rejected();
                    let terminal = effects.iter().find_map(|effect| match effect {
                        hooks::Effect::EndTurn { session, .. } => {
                            self.terminals.get(session).copied()
                        }
                        _ => None,
                    });
                    wired::run_hook_effects(terminal, &effects);
                }
            }
            IoEvent::ContextChanged(editor) => {
                let params = wired::selection_changed(editor);
                if let Some(params) = &params
                    && self.selection.as_ref() != Some(params)
                {
                    wired::notify_selection(None, params);
                }
                if params.is_some() {
                    self.selection = params;
                }
            }
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
