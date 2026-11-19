//! Fyx demo 12: terminal profiles of the staged `fyx:ui` 0.4 (`docs/fyx/wit-0.4/design.md`
//! §B.1, §B.6). The manifest does the work: two `[[fyx_ui.terminal_profiles]]` entries appear in
//! the terminal panel's "+" menu as "<profile> — Terminal profiles demo".
//!
//! - **Git history** runs `git log --graph --oneline --decorate -n 200` in the active worktree.
//! - **Python with a welcome** runs `python -i ${package}/scripts/welcome.py` in the home folder.
//!   `${package}` is the extension's installed folder (`package:files`), so the script ships
//!   with the extension. Python runs any code typed into it, and the menu says so.
//!
//! A profile terminal belongs to the user: the extension cannot read or type into it, and needs
//! no T2 capability. One panel `"terminal-profiles"` (right dock, title "Terminal profiles",
//! icon `terminal`) lists the profiles and how to start them.

use ui_ext_sdk::*;

const PANEL: &str = "terminal-profiles";

/// The profiles as the manifest declares them, for the panel: name and command line.
pub const PROFILES: [(&str, &str); 2] = [
    ("Git history", "git log --graph --oneline --decorate -n 200"),
    (
        "Python with a welcome",
        "python -i ${package}/scripts/welcome.py",
    ),
];

pub struct TerminalProfilesDemo;

impl Extension for TerminalProfilesDemo {
    fn new() -> Self {
        Self
    }

    fn surfaces(&self) -> Vec<Surface> {
        vec![Surface::panel(
            PANEL,
            "Terminal profiles",
            DockPosition::Right,
            Icon::Terminal,
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
            tree.text(TextProps::new("Terminal profiles").bold());
            tree.text(
                TextProps::new(
                    "Start these from the terminal panel's + menu. Their terminals are yours: \
                     this extension cannot read or type into them.",
                )
                .color(ColorToken::TextMuted),
            );
            for (name, command) in PROFILES {
                tree.flex(FlexProps::column().gap(2), |tree| {
                    tree.text(TextProps::new(name));
                    tree.text(
                        TextProps::new(command)
                            .monospace()
                            .color(ColorToken::TextMuted),
                    );
                });
            }
        });
        Ok(tree.finish())
    }
}

#[cfg(not(any(feature = "fyx-ui-0-2", feature = "fyx-ui-0-3")))]
register_extension!(TerminalProfilesDemo, v0_1);
#[cfg(all(feature = "fyx-ui-0-2", not(feature = "fyx-ui-0-3")))]
register_extension!(TerminalProfilesDemo, v0_2);
#[cfg(feature = "fyx-ui-0-3")]
register_extension!(TerminalProfilesDemo);

#[cfg(test)]
mod tests {
    use super::*;

    /// The harness checks the list against the manifest (`fyx_ext_harness`'s
    /// `demos_v0_4.rs`); this checks the panel shows it.
    #[test]
    fn the_panel_lists_the_profiles() {
        let mut demo = TerminalProfilesDemo::new();
        let nodes = demo.mount(PANEL).unwrap();
        let texts: Vec<String> = nodes
            .iter()
            .filter_map(|node| match &node.props {
                Props::Text(text) => Some(text.content.clone()),
                _ => None,
            })
            .collect();
        for (name, command) in PROFILES {
            assert!(texts.iter().any(|text| text == name), "{texts:?}");
            assert!(texts.iter().any(|text| text == command), "{texts:?}");
        }
        assert!(demo.mount("other").is_err());
    }
}
