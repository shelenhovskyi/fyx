//! `fyx:ui` guest with one status-bar item, surface `status` on the right, priority 10, which the
//! demos do not cover. `mount` returns, with fixed ids:
//!
//! | id | parent | props |
//! |---|---|---|
//! | 1 | none | row flex, gap 4 |
//! | 2 | 1 | icon `bell` |
//! | 3 | 1 | text `Pings: 0`, muted |
//! | 4 | 1 | button `Ping`, subtle |
//!
//! `click(4)` answers with `set-props` on 3 reading `Pings: {n}`; other events are `unchanged`.

use ui_ext_sdk::*;

const SURFACE_ID: &str = "status";

#[derive(Default)]
struct StatusItem {
    pings: u32,
    tree: SurfaceTree,
    label: NodeId,
    ping: NodeId,
}

impl StatusItem {
    fn label(&self) -> TextProps {
        TextProps::new(format!("Pings: {}", self.pings)).color(ColorToken::TextMuted)
    }
}

impl Extension for StatusItem {
    fn new() -> Self {
        Self::default()
    }

    fn surfaces(&self) -> Vec<Surface> {
        vec![Surface::status_item(
            SURFACE_ID,
            "Pings",
            StatusBarSide::Right,
            10,
        )]
    }

    fn mount(&mut self, surface_id: &str) -> Result<Vec<Node>, String> {
        if surface_id != SURFACE_ID {
            return Err(format!("unknown surface {surface_id:?}"));
        }
        let mut tree = std::mem::take(&mut self.tree);
        let (mut label, mut ping) = (0, 0);
        let mounted = tree.mount(|tree| {
            tree.flex(FlexProps::row().gap(4), |tree| {
                tree.icon(IconProps::new(Icon::Bell));
                label = tree.text(self.label());
                ping = tree.button(ButtonProps::new("Ping").style(ButtonStyle::Subtle));
            });
        });
        self.tree = tree;
        self.label = label;
        self.ping = ping;
        Ok(mounted?)
    }

    fn handle_event(&mut self, surface_id: &str, event: UiEvent) -> Result<Update, String> {
        if surface_id != SURFACE_ID {
            return Err(format!("unknown surface {surface_id:?}"));
        }
        match event {
            UiEvent::Click(id) if id == self.ping => {
                self.pings += 1;
                let label = self.label();
                self.tree.set_props(self.label, label)?;
                Ok(self.tree.take_update())
            }
            _ => Ok(Update::Unchanged),
        }
    }
}

// The host runs fyx:ui 0.1; the SDK's default world is 0.2.
register_extension!(StatusItem, v0_1);
