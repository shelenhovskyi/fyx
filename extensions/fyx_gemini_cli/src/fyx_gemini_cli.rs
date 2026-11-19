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
//! Built for 0.4, [`companion::Server`] runs on the SDK's `listeners`, `context` and `review`
//! (`wired.rs`): `mcp-call`, `review-decided` and `context-changed` drive it. What the kit still
//! lacks for Gemini is listed in `compat/README.md`.

pub mod companion;
#[cfg(feature = "fyx-ui-0-4")]
mod wired;

#[cfg(test)]
mod compat;

use ui_ext_sdk::*;

const PANEL: &str = "gemini-cli";

pub const FEATURES: [&str; 3] = [
    "Sends your open files, cursor and selection to Gemini as context.",
    "Shows Gemini's edits in agent review; your decision, with your edits, goes back to Gemini.",
    "Closes the review when you answer in the terminal instead.",
];

pub struct GeminiCli {
    server: companion::Server,
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
        }
    }

    #[cfg(feature = "fyx-ui-0-4")]
    fn handle_io(&mut self, event: IoEvent) -> Result<(), String> {
        match event {
            IoEvent::McpCall(call) if call.listener == companion::LISTENER => {
                self.server.call(
                    &mut wired::SdkHost,
                    call.connection,
                    call.call_id,
                    &call.tool,
                    &call.arguments,
                );
            }
            IoEvent::McpInitialized(session) if session.listener == companion::LISTENER => {
                self.server
                    .initialized(&mut wired::SdkHost, session.connection);
            }
            IoEvent::ReviewDecided((proposal, outcome)) => {
                self.server.review_decided(
                    &mut wired::SdkHost,
                    proposal,
                    wired::decision(&outcome),
                );
            }
            IoEvent::ContextChanged(editor) => {
                self.server
                    .context_changed(&mut wired::SdkHost, wired::context_update(editor));
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
