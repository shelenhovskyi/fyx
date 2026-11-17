//! Minimal `fyx:ui` guest for the host tests: a counter with an increment and a reset button and
//! a three-row virtualized list. Node 99 spins forever, to exercise the host's time budget.

use std::cell::Cell;

use ui_ext_sdk::bindings::{
    self, Guest,
    fyx::ui::host::{self, LogLevel},
    fyx::ui::types::{
        Align, ButtonProps, ButtonStyle, DockPosition, FlexDirection, FlexProps, InsertOp,
        ListProps, ListRow, Node, NodeId, PanelDescriptor, Patch, Props, Surface, SurfaceKind,
        TextProps, UiEvent, Update,
    },
};

#[used]
#[unsafe(link_section = "fyx:ui-api-version")]
pub static FYX_UI_API_VERSION: [u8; 6] = [0, 0, 0, 1, 0, 0];

const SURFACE: &str = "counter";
const ROOT: NodeId = 1;
const LABEL: NodeId = 2;
const INCREMENT: NodeId = 3;
const RESET: NodeId = 4;
const LIST: NodeId = 5;
const FIRST_CLICK_NOTE: NodeId = 10;
const SPIN: NodeId = 99;

thread_local! {
    static COUNT: Cell<u32> = const { Cell::new(0) };
}

fn text(content: String) -> Props {
    Props::Text(TextProps {
        content,
        color: None,
        bold: false,
        monospace: false,
    })
}

fn button(label: &str) -> Props {
    Props::Button(ButtonProps {
        label: label.into(),
        icon: None,
        style: ButtonStyle::Default,
        disabled: false,
    })
}

fn label(count: u32) -> Patch {
    Patch::SetProps((LABEL, text(format!("Count: {count}"))))
}

struct Counter;

impl Guest for Counter {
    fn surfaces() -> Vec<Surface> {
        vec![Surface {
            id: SURFACE.into(),
            title: "Counter".into(),
            kind: SurfaceKind::Panel(PanelDescriptor {
                position: DockPosition::Left,
                icon: "hash".into(),
                default_size: 240,
            }),
        }]
    }

    fn mount(surface_id: String) -> Result<Vec<Node>, String> {
        if surface_id != SURFACE {
            return Err(format!("unknown surface {surface_id}"));
        }
        let node = |id, parent, props| Node { id, parent, props };
        Ok(vec![
            node(
                ROOT,
                None,
                Props::Flex(FlexProps {
                    direction: FlexDirection::Column,
                    gap: 4,
                    padding: 8,
                    grow: true,
                    align: Align::Stretch,
                    background: None,
                }),
            ),
            node(LABEL, Some(ROOT), text(format!("Count: {}", COUNT.get()))),
            node(INCREMENT, Some(ROOT), button("Increment")),
            node(RESET, Some(ROOT), button("Reset")),
            node(
                LIST,
                Some(ROOT),
                Props::List(ListProps {
                    row_count: 3,
                    row_height: 22,
                }),
            ),
        ])
    }

    fn handle_event(surface_id: String, event: UiEvent) -> Result<Update, String> {
        if surface_id != SURFACE {
            return Err(format!("unknown surface {surface_id}"));
        }
        match event {
            UiEvent::Click(INCREMENT) => {
                let count = COUNT.get() + 1;
                COUNT.set(count);
                host::log(LogLevel::Info, &format!("count is {count}"));
                let mut patches = vec![label(count)];
                if count == 1 {
                    patches.push(Patch::Insert(InsertOp {
                        node: Node {
                            id: FIRST_CLICK_NOTE,
                            parent: Some(ROOT),
                            props: text("first click".into()),
                        },
                        before: Some(INCREMENT),
                    }));
                }
                Ok(Update::Patches(patches))
            }
            UiEvent::Click(RESET) => {
                COUNT.set(0);
                // Exercises the host import: the note disappears through a pushed update.
                host::push_update(
                    SURFACE,
                    &Update::Patches(vec![Patch::Remove(FIRST_CLICK_NOTE)]),
                );
                Ok(Update::Patches(vec![label(0)]))
            }
            UiEvent::Click(SPIN) => loop {
                std::hint::spin_loop();
            },
            UiEvent::InputChanged(_) | UiEvent::InputSubmitted(_) => Ok(Update::Unchanged),
            UiEvent::Click(id) | UiEvent::RowClick((id, _)) => {
                Err(format!("no handler for node {id}"))
            }
        }
    }

    fn list_rows(surface_id: String, list_node: NodeId, start: u32, count: u32) -> Vec<ListRow> {
        if surface_id != SURFACE || list_node != LIST {
            return Vec::new();
        }
        (start..start.saturating_add(count).min(3))
            .map(|index| ListRow {
                index,
                nodes: vec![Node {
                    id: 1000 + index,
                    parent: None,
                    props: text(format!("Row {index}")),
                }],
            })
            .collect()
    }
}

bindings::export!(Counter);
