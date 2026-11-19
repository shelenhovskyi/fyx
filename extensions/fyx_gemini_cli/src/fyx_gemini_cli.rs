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
//! Host-free like `fyx_claude_code`: the wiring to `mcp-call`, `review-decided` and
//! `context-changed` waits for plan 5.14. The primitives Gemini needs that Claude did not are
//! listed in `compat/README.md`.

pub mod companion;

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
    diffs: companion::Diffs,
}

impl Extension for GeminiCli {
    fn new() -> Self {
        Self {
            diffs: companion::Diffs::default(),
        }
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
            let open = match self.diffs.len() {
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
