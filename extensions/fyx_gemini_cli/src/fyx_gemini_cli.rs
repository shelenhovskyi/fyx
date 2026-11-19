//! `fyx_gemini_cli` (plan 5.16, `docs/fyx/wit-0.4/design.md` "The first extensions"): Gemini
//! CLI in Fyx's terminal, with Fyx as its IDE companion. It is the second tool on the 0.4
//! integration kit, built to show that the kit is general.
//!
//! - **Profile.** "Gemini CLI" in the terminal panel's "+" menu runs `gemini` in the active
//!   worktree, with an account the user picks (`account_tool = "gemini"`).
//! - **Companion** ([`companion`]): the host's `mcp-streamable-http` listener `ide` with a Bearer
//!   token, the discovery file `gemini-ide-server-<pid>-<port>.json` and
//!   `GEMINI_CLI_IDE_SERVER_PORT`. `openDiff` opens a review proposal and answers at once; the
//!   decision goes back as `ide/diffAccepted` or `ide/diffRejected`. `ide/contextUpdate` sends
//!   the open files and the selection.
//! - **Compat test** (`compat`): transcripts of a pinned Gemini CLI version, replayed against
//!   [`companion`].
//!
//! - **Hooks** ([`hooks`]): `gemini-settings/system-defaults.json`, loaded through
//!   `GEMINI_CLI_SYSTEM_DEFAULTS_PATH`, runs `fyx-relay` for Gemini's hooks. `BeforeAgent` begins
//!   an agent turn and `AfterAgent` ends it with the files Gemini wrote; the tool, the waits and
//!   the turn show on the terminal's tab. Every hook is answered at once with exit 0.
//!
//! Built for 0.4, [`companion::Server`] runs on the SDK's `listeners`, `context` and `review`
//! (`wired.rs`): `mcp-call`, `review-decided` and `context-changed` drive it, and
//! `relay-request` drives [`hooks::Sessions`]. What the kit still lacks for Gemini is listed in
//! `compat/README.md`.

pub mod companion;
pub mod hooks;
#[cfg(feature = "fyx-ui-0-4")]
mod wired;

#[cfg(feature = "fyx-ui-0-4")]
use std::collections::BTreeMap;

#[cfg(test)]
mod compat;

use ui_ext_sdk::*;

const PANEL: &str = "gemini-cli";

pub const FEATURES: [&str; 4] = [
    "Sends your open files, cursor and selection to Gemini as context.",
    "Shows Gemini's edits in agent review; your decision, with your edits, goes back to Gemini.",
    "Closes the review when you answer in the terminal instead.",
    "Makes each prompt an agent turn and shows what Gemini does on the terminal's tab.",
];

pub struct GeminiCli {
    server: companion::Server,
    #[cfg_attr(not(feature = "fyx-ui-0-4"), allow(dead_code))]
    sessions: hooks::Sessions,
    /// The terminal each Gemini session's hooks last ran in, for its status.
    #[cfg(feature = "fyx-ui-0-4")]
    terminals: BTreeMap<String, terminals::TerminalHandle>,
}

#[cfg(feature = "fyx-ui-0-4")]
impl GeminiCli {
    /// The host, with the session `openDiff` proposals are named by.
    fn host(&self) -> wired::SdkHost {
        wired::SdkHost::new(self.sessions.current())
    }

    /// One hook, through `fyx-relay`. A hook the extension cannot read is logged; every hook is
    /// answered with [`hooks::ANSWER`], so Gemini never waits on Fyx.
    fn hook(&mut self, request: relay::RelayRequest) {
        let input = hooks::Channel::from_relay(&request.channel)
            .ok_or_else(|| format!("unknown relay channel {:?}", request.channel))
            .and_then(|channel| hooks::HookInput::parse(channel, &request.stdin));
        match input {
            Ok(input) => {
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
                &format!("a Gemini hook was ignored: {error}"),
            ),
        }
        let (exit_code, stdout, stderr) = hooks::ANSWER;
        if let Err(error) = relay::respond(request.id, exit_code, stdout, stderr) {
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

impl Extension for GeminiCli {
    fn new() -> Self {
        // Registered before the listener opens; the host offers them when it does.
        #[cfg(feature = "fyx-ui-0-4")]
        {
            wired::register_tools();
            wired::watch_context();
        }
        Self {
            server: companion::Server::default(),
            sessions: hooks::Sessions::default(),
            #[cfg(feature = "fyx-ui-0-4")]
            terminals: BTreeMap::new(),
        }
    }

    #[cfg(feature = "fyx-ui-0-4")]
    fn handle_io(&mut self, event: IoEvent) -> Result<(), String> {
        match event {
            IoEvent::McpCall(call) if call.listener == companion::LISTENER => {
                let mut host = self.host();
                self.server.call(
                    &mut host,
                    call.connection,
                    call.call_id,
                    &call.tool,
                    &call.arguments,
                );
            }
            IoEvent::McpInitialized(session) if session.listener == companion::LISTENER => {
                let mut host = self.host();
                self.server.initialized(&mut host, session.connection);
            }
            IoEvent::ReviewDecided((proposal, outcome)) => {
                let mut host = self.host();
                self.server
                    .review_decided(&mut host, proposal, wired::decision(&outcome));
            }
            IoEvent::ContextChanged(editor) => {
                let mut host = self.host();
                self.server
                    .context_changed(&mut host, wired::context_update(editor));
            }
            IoEvent::RelayRequest((id, channel, terminal, argv, stdin)) => {
                self.hook(relay::RelayRequest {
                    id,
                    channel,
                    terminal,
                    argv,
                    stdin,
                });
            }
            _ => {}
        }
        Ok(())
    }

    fn surfaces(&self) -> Vec<Surface> {
        vec![Surface::panel(
            PANEL,
            "Gemini CLI",
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
            tree.text(TextProps::new("Gemini CLI").bold());
            tree.text(
                TextProps::new(
                    "Start Gemini CLI from the terminal panel's + menu, then run /ide enable \
                     in it. Its terminal is yours, and this extension cannot read it.",
                )
                .color(ColorToken::TextMuted),
            );
            for feature in FEATURES {
                tree.text(TextProps::new(feature));
            }
            let open = match self.server.open_diffs() {
                0 => "No edit is waiting for review.".to_owned(),
                1 => "1 edit is waiting for review.".to_owned(),
                count => format!("{count} edits are waiting for review."),
            };
            tree.text(TextProps::new(open).color(ColorToken::TextMuted));
        });
        Ok(tree.finish())
    }
}

#[cfg(not(any(feature = "fyx-ui-0-2", feature = "fyx-ui-0-3")))]
register_extension!(GeminiCli, v0_1);
#[cfg(all(feature = "fyx-ui-0-2", not(feature = "fyx-ui-0-3")))]
register_extension!(GeminiCli, v0_2);
#[cfg(feature = "fyx-ui-0-3")]
register_extension!(GeminiCli);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_says_what_it_connects() {
        let mut extension = GeminiCli::new();
        let texts: Vec<String> = extension
            .mount(PANEL)
            .unwrap()
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
                .any(|text| text == "No edit is waiting for review.")
        );
        assert!(extension.mount("other").is_err());
    }
}
