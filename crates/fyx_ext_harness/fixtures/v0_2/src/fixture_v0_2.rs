//! `fyx:ui@0.2.0` guest for the `fyx_ext_harness` tests, written with the SDK so the tests also
//! cover its 0.2 export. Ids are fixed, so the tests can name nodes.
//!
//! - `surfaces()` declares the panel `main` and the modal `picker`, and calls `add-surface` for
//!   the status item `boot`.
//! - `main` (ids below): text 3 reports the result of the last host call, `ok` or the error code.
//!
//!   | id | node | on click |
//!   |---|---|---|
//!   | 1 | column | |
//!   | 2 | text `ticks: N` | |
//!   | 3 | text `status: …` | |
//!   | 4 | button `Tick` | `request-tick(main, 10)` |
//!   | 5 | button `Open picker` | `show-surface(picker)` |
//!   | 6 | input | typing `open` calls `show-surface(picker)`, without a gesture |
//!   | 7 | button `Add clock` | `add-surface(status-item clock)` |
//!   | 8 | button `Remove clock` | `remove-surface(clock)`, then `push-update(clock)` |
//!   | 9 | button `Place note` | `place-decoration(note, src/lib.rs:3)` |
//!   | 10 | button `Remove note` | `remove-decoration(note)` |
//!   | 11 | list of 3 rows, row `i` reads `row i` with indent `i` | |
//!   | 12 | button `Refresh rows` | answers `invalidate-rows(11, 1..2)` |
//!   | 13 | button `Failing ticks` | every later tick re-requests itself and fails |
//!   | 14 | button `Open later` | `request-tick(main, 100)`; that tick shows the picker |
//!   | 15 | button `Place bad note` | places `bad`, whose tree has an orphan; `mount(bad)` fails |
//!   | 16 | button `Flood` | 70 pushes to `main`; text 3 reads `status: ok*64 limit-exceeded*6` |
//!
//! - `tick(main)` counts into text 2 and does not re-request.
//! - `picker`: column 1, secret input 2, list 3 of 5 rows (`item i`), text 4 `selected: …`, button
//!   5 `Close` with a tooltip. `row-selected` updates text 4; `input-submitted` hides the picker
//!   and pushes `status: picked N` to `main`; `Close` hides it; `dismissed` pushes
//!   `status: dismissed <reason>` to `main` and answers a `set-props`, which the host ignores.
//! - `note`: column 1, text 2 `note`, button 3 `Ack`, which answers text 2 `acked`.

use ui_ext_sdk::*;

const MAIN: &str = "main";
const PICKER: &str = "picker";
const NOTE: &str = "note";
const BAD_NOTE: &str = "bad";

const TICKS: NodeId = 2;
const STATUS: NodeId = 3;
const TICK: NodeId = 4;
const OPEN: NodeId = 5;
const INPUT: NodeId = 6;
const ADD_CLOCK: NodeId = 7;
const REMOVE_CLOCK: NodeId = 8;
const PLACE_NOTE: NodeId = 9;
const REMOVE_NOTE: NodeId = 10;
const LIST: NodeId = 11;
const REFRESH: NodeId = 12;
const FAILING_TICKS: NodeId = 13;
const OPEN_LATER: NodeId = 14;
const PLACE_BAD_NOTE: NodeId = 15;
const FLOOD: NodeId = 16;

const PICKER_INPUT: NodeId = 2;
const PICKER_LIST: NodeId = 3;
const PICKER_SELECTED: NodeId = 4;
const PICKER_CLOSE: NodeId = 5;

const NOTE_TEXT: NodeId = 2;
const NOTE_ACK: NodeId = 3;

#[derive(Default)]
struct Fixture {
    ticks: u32,
    failing_ticks: bool,
    open_on_tick: bool,
    selected: Option<u32>,
}

fn node(id: NodeId, parent: Option<NodeId>, props: impl Into<Props>) -> Node {
    Node {
        id,
        parent,
        props: props.into(),
    }
}

fn outcome(result: Result<(), HostError>) -> String {
    match result {
        Ok(()) => "ok".into(),
        Err(error) => match error.code {
            HostErrorCode::UnknownSurface => "unknown-surface",
            HostErrorCode::LimitExceeded => "limit-exceeded",
            HostErrorCode::InvalidArgument => "invalid-argument",
            HostErrorCode::NotPermitted => "not-permitted",
            HostErrorCode::Unavailable => "unavailable",
            HostErrorCode::Denied => "denied",
        }
        .into(),
    }
}

fn set_status(status: &str) -> Update {
    let mut patches = PatchSet::new();
    patches.set_props(STATUS, TextProps::new(format!("status: {status}")));
    patches.into_update()
}

fn note_location() -> DecorationLocation {
    DecorationLocation {
        target: DecorationTarget::Path("src/lib.rs".into()),
        line: 3,
        placement: DecorationPlacement::Below,
        height_lines: Some(2),
    }
}

fn note_tree() -> Vec<Node> {
    vec![
        node(1, None, FlexProps::column()),
        node(NOTE_TEXT, Some(1), TextProps::new("note")),
        node(NOTE_ACK, Some(1), ButtonProps::new("Ack")),
    ]
}

impl Fixture {
    fn main_tree(&self) -> Vec<Node> {
        let button = |id, label: &str| node(id, Some(1), ButtonProps::new(label));
        vec![
            node(1, None, FlexProps::column().gap(4)),
            node(
                TICKS,
                Some(1),
                TextProps::new(format!("ticks: {}", self.ticks)),
            ),
            node(STATUS, Some(1), TextProps::new("status: ready")),
            button(TICK, "Tick"),
            button(OPEN, "Open picker"),
            node(INPUT, Some(1), InputProps::new("")),
            button(ADD_CLOCK, "Add clock"),
            button(REMOVE_CLOCK, "Remove clock"),
            button(PLACE_NOTE, "Place note"),
            button(REMOVE_NOTE, "Remove note"),
            node(LIST, Some(1), ListProps::new(3, 20)),
            button(REFRESH, "Refresh rows"),
            button(FAILING_TICKS, "Failing ticks"),
            button(OPEN_LATER, "Open later"),
            button(PLACE_BAD_NOTE, "Place bad note"),
            button(FLOOD, "Flood"),
        ]
    }

    fn picker_tree(&self) -> Vec<Node> {
        vec![
            node(1, None, FlexProps::column()),
            node(
                PICKER_INPUT,
                Some(1),
                InputProps::new("").placeholder("Filter").secret(),
            ),
            node(PICKER_LIST, Some(1), ListProps::new(5, 20).selected(None)),
            node(PICKER_SELECTED, Some(1), TextProps::new("selected: none")),
            node(
                PICKER_CLOSE,
                Some(1),
                ButtonProps::new("Close")
                    .icon(Icon::Close)
                    .tooltip("Close the picker"),
            ),
        ]
    }

    fn main_event(&mut self, event: UiEvent) -> Result<Update, String> {
        let status = match event {
            UiEvent::Click(TICK) => outcome(request_tick(MAIN, 10)),
            UiEvent::Click(OPEN) => outcome(show_surface(PICKER)),
            UiEvent::InputChanged((INPUT, text)) if text == "open" => outcome(show_surface(PICKER)),
            UiEvent::Click(ADD_CLOCK) => outcome(add_surface(&Surface::status_item(
                "clock",
                "Clock",
                StatusBarSide::Right,
                0,
            ))),
            UiEvent::Click(REMOVE_CLOCK) => {
                let removed = outcome(remove_surface("clock"));
                let pushed = outcome(push_update("clock", &Update::Unchanged));
                format!("{removed}, push {pushed}")
            }
            UiEvent::Click(PLACE_NOTE) => {
                outcome(place_decoration(NOTE, &note_location(), &note_tree()))
            }
            UiEvent::Click(REMOVE_NOTE) => outcome(remove_decoration(NOTE)),
            UiEvent::Click(REFRESH) => {
                let mut patches = PatchSet::new();
                patches.invalidate_rows(LIST, Some(RowRange { start: 1, count: 1 }));
                return Ok(patches.into_update());
            }
            UiEvent::Click(FAILING_TICKS) => {
                self.failing_ticks = true;
                outcome(request_tick(MAIN, 100))
            }
            UiEvent::Click(OPEN_LATER) => {
                self.open_on_tick = true;
                outcome(request_tick(MAIN, 100))
            }
            UiEvent::Click(PLACE_BAD_NOTE) => {
                let orphan = node(1, Some(99), TextProps::new("orphan"));
                outcome(place_decoration(BAD_NOTE, &note_location(), &[orphan]))
            }
            UiEvent::Click(FLOOD) => {
                let mut results: Vec<(String, u32)> = Vec::new();
                for _ in 0..70 {
                    let result = outcome(push_update(MAIN, &Update::Unchanged));
                    match results.last_mut() {
                        Some((last, count)) if *last == result => *count += 1,
                        _ => results.push((result, 1)),
                    }
                }
                results
                    .iter()
                    .map(|(result, count)| format!("{result}*{count}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            }
            _ => return Ok(Update::Unchanged),
        };
        Ok(set_status(&status))
    }

    fn picker_event(&mut self, event: UiEvent) -> Result<Update, String> {
        match event {
            UiEvent::RowSelected((PICKER_LIST, row)) => {
                self.selected = Some(row);
                let mut patches = PatchSet::new();
                patches.set_props(PICKER_SELECTED, TextProps::new(format!("selected: {row}")));
                Ok(patches.into_update())
            }
            UiEvent::InputSubmitted((PICKER_INPUT, _)) => {
                hide_surface(PICKER).map_err(|error| error.message)?;
                let picked = self
                    .selected
                    .map_or_else(|| "nothing".to_string(), |row| row.to_string());
                push_update(MAIN, &set_status(&format!("picked {picked}")))
                    .map_err(|error| error.message)?;
                Ok(Update::Unchanged)
            }
            UiEvent::Click(PICKER_CLOSE) => {
                hide_surface(PICKER).map_err(|error| error.message)?;
                Ok(Update::Unchanged)
            }
            UiEvent::Dismissed(reason) => {
                let reason = match reason {
                    DismissReason::Escape => "escape",
                    DismissReason::FocusLost => "focus-lost",
                    DismissReason::Replaced => "replaced",
                    DismissReason::WindowClosed => "window-closed",
                };
                push_update(MAIN, &set_status(&format!("dismissed {reason}")))
                    .map_err(|error| error.message)?;
                let mut patches = PatchSet::new();
                patches.set_props(PICKER_SELECTED, TextProps::new("ignored"));
                Ok(patches.into_update())
            }
            _ => Ok(Update::Unchanged),
        }
    }
}

impl Extension for Fixture {
    fn new() -> Self {
        Self::default()
    }

    fn surfaces(&self) -> Vec<Surface> {
        if let Err(error) = add_surface(&Surface::status_item(
            "boot",
            "Boot",
            StatusBarSide::Left,
            0,
        )) {
            log(
                LogLevel::Error,
                &format!("add-surface(boot): {}", error.message),
            );
        }
        vec![
            Surface::panel(MAIN, "Main", DockPosition::Right, Icon::Sparkle, 300),
            Surface::modal(PICKER, "Pick one", ModalWidth::Medium),
        ]
    }

    fn mount(&mut self, surface_id: &str) -> Result<Vec<Node>, String> {
        match surface_id {
            MAIN => Ok(self.main_tree()),
            PICKER => {
                self.selected = None;
                Ok(self.picker_tree())
            }
            NOTE => Ok(note_tree()),
            "boot" | "clock" => Ok(vec![node(1, None, TextProps::new(surface_id))]),
            _ => Err(format!("cannot mount {surface_id:?}")),
        }
    }

    fn handle_event(&mut self, surface_id: &str, event: UiEvent) -> Result<Update, String> {
        match surface_id {
            MAIN => self.main_event(event),
            PICKER => self.picker_event(event),
            NOTE => match event {
                UiEvent::Click(NOTE_ACK) => {
                    let mut patches = PatchSet::new();
                    patches.set_props(NOTE_TEXT, TextProps::new("acked"));
                    Ok(patches.into_update())
                }
                _ => Ok(Update::Unchanged),
            },
            _ => Err(format!("unknown surface {surface_id:?}")),
        }
    }

    fn list_rows(
        &mut self,
        surface_id: &str,
        list_node: NodeId,
        start: u32,
        count: u32,
    ) -> Vec<ListRow> {
        let (row_count, label) = match (surface_id, list_node) {
            (MAIN, LIST) => (3, "row"),
            (PICKER, PICKER_LIST) => (5, "item"),
            _ => return Vec::new(),
        };
        list_rows(start, count, row_count, |index, tree| {
            tree.text(TextProps::new(format!("{label} {index}")));
        })
        .into_iter()
        .map(|row| {
            let indent = if label == "row" { row.index as u8 } else { 0 };
            row.with_indent(indent)
        })
        .collect()
    }

    fn tick(&mut self, surface_id: &str) -> Result<Update, String> {
        if surface_id != MAIN {
            return Err(format!("no ticks for {surface_id:?}"));
        }
        if self.failing_ticks {
            request_tick(MAIN, 100).map_err(|error| error.message)?;
            return Err("tick failed".into());
        }
        self.ticks += 1;
        let mut patches = PatchSet::new();
        patches.set_props(TICKS, TextProps::new(format!("ticks: {}", self.ticks)));
        if self.open_on_tick {
            self.open_on_tick = false;
            let shown = outcome(show_surface(PICKER));
            patches.set_props(STATUS, TextProps::new(format!("status: {shown}")));
        }
        Ok(patches.into_update())
    }
}

register_extension!(Fixture, v0_2);
