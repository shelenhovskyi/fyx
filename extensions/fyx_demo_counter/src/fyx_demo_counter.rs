//! Fyx demo 1: a counter panel with a virtualized list.
//!
//! Host integration tests rely on this behaviour, so keep it stable:
//!
//! - One surface, id `"counter"`: a right dock panel titled "Counter", icon `hash`, default size
//!   320.
//! - The first `mount("counter")` returns 7 nodes, parents first, with these ids:
//!
//!   | id | parent | props |
//!   |---|---|---|
//!   | 1 | none | column flex, gap 8, padding 8, grow |
//!   | 2 | 1 | text `Count: 0`, bold |
//!   | 3 | 1 | row flex, gap 4 |
//!   | 4 | 3 | button `Decrement`, icon `dash`, disabled while the count is 0 |
//!   | 5 | 3 | button `Increment`, icon `plus`, filled |
//!   | 6 | 3 | button `Reset`, subtle, disabled while the count is 0 |
//!   | 7 | 1 | list, `row-count` = count, `row-height` 24 |
//!
//! - Ids are fixed for the first mount only. The `SurfaceTree` id allocator survives remounts (a
//!   panel reopen, or the host remounting after it rejected an update), so the second mount
//!   returns the same tree with ids 8 to 14, the third 15 to 21, and so on; events and
//!   `list-rows` then use those ids. The count survives a remount. Ids below are the first
//!   mount's.
//! - `handle-event` answers `click(4|5|6)` with `patches` containing only `set-props` for nodes
//!   whose props changed, never `replace`. The first increment from 0 sets props on 2, 4, 6 and 7;
//!   the second only on 2 and 7. A click that changes nothing (decrement at 0) returns
//!   `unchanged`, as do other events. An unknown surface id is an error.
//! - `list-rows("counter", 7, start, count)` returns rows `start..min(start + count, count)`. Row
//!   `i` is a subtree of a row flex (id `0x8000_0000`, no parent) holding one text node
//!   (`0x8000_0001`) reading `Row {i + 1} of {count}`, so every row reflects the current count.

use ui_ext_sdk::*;

const SURFACE_ID: &str = "counter";
const ROW_HEIGHT: u16 = 24;

struct CounterNodes {
    label: NodeId,
    decrement: NodeId,
    increment: NodeId,
    reset: NodeId,
    list: NodeId,
}

#[derive(Default)]
struct Counter {
    count: u32,
    tree: SurfaceTree,
    nodes: Option<CounterNodes>,
}

impl Counter {
    fn label(&self) -> TextProps {
        TextProps::new(format!("Count: {}", self.count)).bold()
    }

    fn decrement(&self) -> ButtonProps {
        ButtonProps::new("Decrement")
            .icon("dash")
            .disabled(self.count == 0)
    }

    fn increment(&self) -> ButtonProps {
        ButtonProps::new("Increment")
            .icon("plus")
            .style(ButtonStyle::Filled)
    }

    fn reset(&self) -> ButtonProps {
        ButtonProps::new("Reset")
            .style(ButtonStyle::Subtle)
            .disabled(self.count == 0)
    }

    fn list(&self) -> ListProps {
        ListProps::new(self.count, ROW_HEIGHT)
    }

    fn sync(&mut self) -> Result<Update, String> {
        let Some(nodes) = &self.nodes else {
            return Err(TreeError::NotMounted.into());
        };
        self.tree.set_props(nodes.label, self.label())?;
        self.tree.set_props(nodes.decrement, self.decrement())?;
        self.tree.set_props(nodes.increment, self.increment())?;
        self.tree.set_props(nodes.reset, self.reset())?;
        self.tree.set_props(nodes.list, self.list())?;
        Ok(self.tree.take_update())
    }
}

fn check_surface(surface_id: &str) -> Result<(), String> {
    if surface_id == SURFACE_ID {
        Ok(())
    } else {
        Err(format!("unknown surface {surface_id:?}"))
    }
}

impl Extension for Counter {
    fn new() -> Self {
        Self::default()
    }

    fn surfaces(&self) -> Vec<Surface> {
        vec![Surface::panel(
            SURFACE_ID,
            "Counter",
            DockPosition::Right,
            "hash",
            320,
        )]
    }

    fn mount(&mut self, surface_id: &str) -> Result<Vec<Node>, String> {
        check_surface(surface_id)?;
        let mut tree = std::mem::take(&mut self.tree);
        let mut nodes = CounterNodes {
            label: 0,
            decrement: 0,
            increment: 0,
            reset: 0,
            list: 0,
        };
        let mounted = tree.mount(|tree| {
            tree.flex(FlexProps::column().gap(8).padding(8).grow(), |tree| {
                nodes.label = tree.text(self.label());
                tree.flex(FlexProps::row().gap(4), |tree| {
                    nodes.decrement = tree.button(self.decrement());
                    nodes.increment = tree.button(self.increment());
                    nodes.reset = tree.button(self.reset());
                });
                nodes.list = tree.list(self.list());
            });
        });
        self.tree = tree;
        let mounted = mounted?;
        self.nodes = Some(nodes);
        Ok(mounted)
    }

    fn handle_event(&mut self, surface_id: &str, event: UiEvent) -> Result<Update, String> {
        check_surface(surface_id)?;
        let Some(nodes) = &self.nodes else {
            return Err(TreeError::NotMounted.into());
        };
        let UiEvent::Click(id) = event else {
            return Ok(Update::Unchanged);
        };
        if id == nodes.increment {
            self.count = self.count.saturating_add(1);
        } else if id == nodes.decrement {
            self.count = self.count.saturating_sub(1);
        } else if id == nodes.reset {
            self.count = 0;
        } else {
            return Ok(Update::Unchanged);
        }
        self.sync()
    }

    fn list_rows(
        &mut self,
        surface_id: &str,
        list_node: NodeId,
        start: u32,
        count: u32,
    ) -> Vec<ListRow> {
        let is_counter_list = self
            .nodes
            .as_ref()
            .is_some_and(|nodes| nodes.list == list_node);
        if surface_id != SURFACE_ID || !is_counter_list {
            log(
                LogLevel::Warn,
                &format!("list-rows for unknown list {list_node} on {surface_id:?}"),
            );
            return Vec::new();
        }
        let total = self.count;
        list_rows(start, count, total, |index, tree| {
            tree.flex(FlexProps::row().padding(4).align(Align::Center), |tree| {
                tree.text(format!("Row {} of {total}", index + 1));
            });
        })
    }
}

register_extension!(Counter);

#[cfg(test)]
mod tests {
    use super::*;

    fn set_props_ids(update: Update) -> Vec<NodeId> {
        let Update::Patches(patches) = update else {
            panic!("expected patches, got {update:?}");
        };
        patches
            .into_iter()
            .map(|patch| match patch {
                Patch::SetProps((id, _)) => id,
                patch => panic!("expected only set-props, got {patch:?}"),
            })
            .collect()
    }

    #[test]
    fn behaves_as_documented() {
        let mut counter = Counter::new();
        assert_eq!(counter.surfaces()[0].id, "counter");

        let nodes = counter.mount("counter").expect("mount");
        let shape = nodes
            .iter()
            .map(|node| (node.id, node.parent))
            .collect::<Vec<_>>();
        assert_eq!(
            shape,
            [
                (1, None),
                (2, Some(1)),
                (3, Some(1)),
                (4, Some(3)),
                (5, Some(3)),
                (6, Some(3)),
                (7, Some(1)),
            ]
        );
        assert_eq!(nodes[1].props, TextProps::new("Count: 0").bold().into());
        assert_eq!(nodes[6].props, ListProps::new(0, 24).into());

        let click = |counter: &mut Counter, id| {
            counter
                .handle_event("counter", UiEvent::Click(id))
                .expect("click")
        };
        assert_eq!(click(&mut counter, 4), Update::Unchanged);
        assert_eq!(set_props_ids(click(&mut counter, 5)), [2, 4, 6, 7]);
        assert_eq!(set_props_ids(click(&mut counter, 5)), [2, 7]);

        let rows = counter.list_rows("counter", 7, 1, 10);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].index, 1);
        assert_eq!(rows[0].nodes[0].id, ROW_NODE_ID_BASE);
        assert_eq!(rows[0].nodes[1].props, TextProps::new("Row 2 of 2").into());

        assert_eq!(set_props_ids(click(&mut counter, 6)), [2, 4, 6, 7]);
        assert_eq!(click(&mut counter, 6), Update::Unchanged);
        assert!(counter.list_rows("counter", 7, 0, 10).is_empty());
        assert!(counter.handle_event("other", UiEvent::Click(5)).is_err());

        click(&mut counter, 5);
        let remounted = counter.mount("counter").expect("second mount");
        let ids = remounted.iter().map(|node| node.id).collect::<Vec<_>>();
        assert_eq!(ids, [8, 9, 10, 11, 12, 13, 14]);
        assert_eq!(remounted[1].props, TextProps::new("Count: 1").bold().into());
        assert_eq!(click(&mut counter, 5), Update::Unchanged);
        assert_eq!(set_props_ids(click(&mut counter, 12)), [9, 14]);
        assert_eq!(counter.list_rows("counter", 14, 0, 10).len(), 2);
        assert!(counter.list_rows("counter", 7, 0, 10).is_empty());
    }
}
