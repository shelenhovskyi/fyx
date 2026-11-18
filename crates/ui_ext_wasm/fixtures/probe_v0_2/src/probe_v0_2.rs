//! `fyx:ui@0.2.0` guest for the host tests: it runs a script of host calls and reports what each
//! one returned, so one component covers every 0.2 import.
//!
//! - `handle-event(_, input-changed(100, script))` stores `script` and answers `unchanged`.
//! - Every other `handle-event`, and `tick`, run the stored script and answer `replace` with one
//!   text node (id 1) whose content is the report: the event, then one result per command, joined
//!   with ` | `. `mount` does the same when a script is stored, and otherwise returns a tree with
//!   every 0.2 prop (tooltip, frozen icons, secret input, selected row). `list-rows` returns rows
//!   whose `indent` is their index, with the report in the first row.
//! - `surfaces()` declares the panel `probe` and calls `add-surface` for the status item `boot`,
//!   which the host applies after the static surfaces.
//!
//! Script commands, separated by `;`:
//! `push <surface> unchanged|remove <id>|patches <n>|replace <n>|invalidate <list>|rich`,
//! `tick <surface> <ms>`, `cancel <surface>`, `add panel|status|modal <id>`, `remove <id>`,
//! `show <id>`, `hide <id>`, `place <id> <path> <line> [<nodes>]`, `unplace <id>`, `log <text>`,
//! `busy <ms>` (computes for that long), `repeat <n> <command>` (`#` in the command becomes the iteration index; results run-length
//! encoded, `ok*64,limit-exceeded*6`), `fail` (answers
//! `err` with the report so far), `spin`, `sleep <ms>`.

use std::cell::RefCell;

wit_bindgen::generate!({
    path: "../../../ui_ext/wit/since_v0.2.0",
    world: "ui-extension",
});

use fyx::ui::decorations;
use fyx::ui::host::{self, HostError, HostErrorCode, LogLevel};
use fyx::ui::types::*;

/// What `register_extension!` emits for 0.1 components: three big-endian `u16`s.
#[cfg(target_arch = "wasm32")]
#[unsafe(link_section = "fyx:ui-api-version")]
#[used]
static FYX_UI_API_VERSION: [u8; 6] = [0, 0, 0, 2, 0, 0];

const SURFACE_ID: &str = "probe";
const SET_SCRIPT: NodeId = 100;

thread_local! {
    static SCRIPT: RefCell<String> = const { RefCell::new(String::new()) };
}

struct Probe;

fn text(id: NodeId, parent: Option<NodeId>, content: &str) -> Node {
    Node {
        id,
        parent,
        props: Props::Text(TextProps {
            content: content.into(),
            color: None,
            bold: false,
            monospace: false,
        }),
    }
}

fn flex(id: NodeId) -> Node {
    Node {
        id,
        parent: None,
        props: Props::Flex(FlexProps {
            direction: FlexDirection::Column,
            gap: 0,
            padding: 0,
            grow: false,
            align: Align::Start,
            background: None,
        }),
    }
}

/// A tree with every prop 0.2 added.
fn rich_tree() -> Vec<Node> {
    vec![
        flex(1),
        Node {
            id: 2,
            parent: Some(1),
            props: Props::Button(ButtonProps {
                label: "Accept".into(),
                icon: Some(Icon::CheckDouble),
                style: ButtonStyle::Filled,
                disabled: false,
                tooltip: Some("Accept every hunk".into()),
            }),
        },
        Node {
            id: 3,
            parent: Some(1),
            props: Props::Input(InputProps {
                value: "hunter2".into(),
                placeholder: "Token".into(),
                secret: true,
            }),
        },
        Node {
            id: 4,
            parent: Some(1),
            props: Props::List(ListProps {
                row_count: 10,
                row_height: 22,
                selected: Some(2),
            }),
        },
        Node {
            id: 5,
            parent: Some(1),
            props: Props::Icon(IconProps {
                name: Icon::Spinner,
                color: Some(ColorToken::TextMuted),
            }),
        },
    ]
}

fn code(error: &HostError) -> &'static str {
    match error.code {
        HostErrorCode::UnknownSurface => "unknown-surface",
        HostErrorCode::LimitExceeded => "limit-exceeded",
        HostErrorCode::InvalidArgument => "invalid-argument",
        HostErrorCode::NotPermitted => "not-permitted",
        HostErrorCode::Unavailable => "unavailable",
    }
}

fn outcome(result: Result<(), HostError>) -> String {
    match result {
        Ok(()) => "ok".into(),
        Err(error) => code(&error).into(),
    }
}

fn surface(kind: &str, id: &str) -> Result<Surface, String> {
    let kind = match kind {
        "panel" => SurfaceKind::Panel(PanelDescriptor {
            position: DockPosition::Right,
            icon: Icon::Box,
            default_size: 300,
        }),
        "status" => SurfaceKind::StatusItem(StatusItemDescriptor {
            side: StatusBarSide::Right,
            priority: 0,
        }),
        "modal" => SurfaceKind::Modal(ModalDescriptor {
            width: ModalWidth::Medium,
        }),
        other => return Err(format!("unknown kind {other}")),
    };
    Ok(Surface {
        id: id.into(),
        title: id.into(),
        kind,
    })
}

fn update(words: &[&str]) -> Result<Update, String> {
    let number = |index: usize| -> Result<u32, String> {
        words
            .get(index)
            .ok_or("missing number")?
            .parse()
            .map_err(|error| format!("{error}"))
    };
    Ok(match words.first().copied() {
        Some("unchanged") => Update::Unchanged,
        Some("remove") => Update::Patches(vec![Patch::Remove(number(1)?)]),
        Some("patches") => Update::Patches((0..number(1)?).map(Patch::Remove).collect()),
        Some("replace") => {
            Update::Replace((0..number(1)?).map(|id| text(id, None, "node")).collect())
        }
        Some("invalidate") => Update::Patches(vec![Patch::InvalidateRows(InvalidateRowsOp {
            target: number(1)?,
            rows: Some(RowRange { start: 2, count: 3 }),
        })]),
        Some("rich") => Update::Replace(rich_tree()),
        other => return Err(format!("unknown update {other:?}")),
    })
}

/// Runs one command; `Err` stops the script.
fn run_command(words: &[&str]) -> Result<String, String> {
    let arg = |index: usize| -> Result<&str, String> {
        words
            .get(index)
            .copied()
            .ok_or_else(|| format!("{words:?}: missing argument {index}"))
    };
    let number = |index: usize| -> Result<u32, String> {
        arg(index)?.parse().map_err(|error| format!("{error}"))
    };
    Ok(match arg(0)? {
        "push" => outcome(host::push_update(arg(1)?, &update(&words[2..])?)),
        "tick" => outcome(host::request_tick(arg(1)?, number(2)?)),
        "cancel" => {
            host::cancel_tick(arg(1)?);
            "done".into()
        }
        "add" => outcome(host::add_surface(&surface(arg(1)?, arg(2)?)?)),
        "remove" => outcome(host::remove_surface(arg(1)?)),
        "show" => outcome(host::show_surface(arg(1)?)),
        "hide" => outcome(host::hide_surface(arg(1)?)),
        "place" => {
            let nodes = match words.get(4) {
                Some(count) => count.parse().map_err(|error| format!("{error}"))?,
                None => 1,
            };
            let location = DecorationLocation {
                path: arg(2)?.into(),
                line: number(3)?,
                placement: DecorationPlacement::Below,
                height_lines: Some(2),
            };
            let nodes: Vec<Node> = (0..nodes).map(|id| text(id, None, "note")).collect();
            outcome(decorations::place_decoration(arg(1)?, &location, &nodes))
        }
        "unplace" => outcome(decorations::remove_decoration(arg(1)?)),
        "log" => {
            host::log(LogLevel::Info, &words[1..].join(" "));
            "done".into()
        }
        "repeat" => {
            let mut runs: Vec<(String, usize)> = Vec::new();
            for index in 0..number(1)? {
                let words: Vec<String> = words[2..]
                    .iter()
                    .map(|word| word.replace('#', &index.to_string()))
                    .collect();
                let words: Vec<&str> = words.iter().map(String::as_str).collect();
                let result = run_command(&words)?;
                match runs.last_mut() {
                    Some((last, count)) if *last == result => *count += 1,
                    _ => runs.push((result, 1)),
                }
            }
            runs.iter()
                .map(|(result, count)| format!("{result}*{count}"))
                .collect::<Vec<_>>()
                .join(",")
        }
        "spin" => loop {
            std::hint::spin_loop();
        },
        "sleep" => {
            std::thread::sleep(std::time::Duration::from_millis(number(1)?.into()));
            "done".into()
        }
        "busy" => {
            let until =
                std::time::Instant::now() + std::time::Duration::from_millis(number(1)?.into());
            while std::time::Instant::now() < until {
                std::hint::spin_loop();
            }
            "done".into()
        }
        "fail" => return Err("fail".into()),
        other => return Err(format!("unknown command {other}")),
    })
}

/// The report: `event`, then one result per command of the stored script.
fn run_script(event: String) -> Result<String, String> {
    let script = SCRIPT.with(|script| script.borrow().clone());
    let mut report = vec![event];
    for command in script.split(';') {
        let words: Vec<&str> = command.split_whitespace().collect();
        if words.is_empty() {
            continue;
        }
        match run_command(&words) {
            Ok(result) => report.push(result),
            Err(error) => {
                report.push(error);
                return Err(report.join(" | "));
            }
        }
    }
    Ok(report.join(" | "))
}

fn report_update(event: String) -> Result<Update, String> {
    run_script(event).map(|report| Update::Replace(vec![text(1, None, &report)]))
}

fn describe(event: &UiEvent) -> String {
    match event {
        UiEvent::Click(id) => format!("click {id}"),
        UiEvent::InputChanged((id, value)) => format!("input-changed {id} {value}"),
        UiEvent::InputSubmitted((id, value)) => format!("input-submitted {id} {value}"),
        UiEvent::RowClick((id, row)) => format!("row-click {id} {row}"),
        UiEvent::RowSelected((id, row)) => format!("row-selected {id} {row}"),
        UiEvent::Dismissed(reason) => format!(
            "dismissed {}",
            match reason {
                DismissReason::Escape => "escape",
                DismissReason::FocusLost => "focus-lost",
                DismissReason::Replaced => "replaced",
                DismissReason::WindowClosed => "window-closed",
            }
        ),
    }
}

impl Guest for Probe {
    fn surfaces() -> Vec<Surface> {
        // Applied after the static surfaces; refused quietly without `ui:status-bar`.
        host::add_surface(&surface("status", "boot").expect("known kind")).ok();
        vec![surface("panel", SURFACE_ID).expect("known kind")]
    }

    fn mount(surface_id: String) -> Result<Vec<Node>, String> {
        if SCRIPT.with(|script| script.borrow().is_empty()) {
            return Ok(rich_tree());
        }
        run_script(format!("mount {surface_id}")).map(|report| vec![text(1, None, &report)])
    }

    fn handle_event(surface_id: String, event: UiEvent) -> Result<Update, String> {
        if let UiEvent::InputChanged((SET_SCRIPT, script)) = &event {
            SCRIPT.with(|stored| *stored.borrow_mut() = script.clone());
            return Ok(Update::Unchanged);
        }
        report_update(format!("{surface_id}: {}", describe(&event)))
    }

    fn list_rows(surface_id: String, list_node: NodeId, start: u32, count: u32) -> Vec<ListRow> {
        let report =
            run_script(format!("rows {surface_id} {list_node}")).unwrap_or_else(|error| error);
        (start..start.saturating_add(count).min(64))
            .map(|index| ListRow {
                index,
                indent: index as u8,
                nodes: vec![text(
                    1,
                    None,
                    &if index == start {
                        report.clone()
                    } else {
                        format!("row {index}")
                    },
                )],
            })
            .collect()
    }

    fn tick(surface_id: String) -> Result<Update, String> {
        report_update(format!("tick {surface_id}"))
    }
}

export!(Probe);
