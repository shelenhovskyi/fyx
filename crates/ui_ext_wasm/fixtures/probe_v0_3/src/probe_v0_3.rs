//! `fyx:ui@0.3.0` guest for the host tests of the staged 0.3 world: the `images` interface
//! (design.md section 12, plan 4.10), image nodes, and an interface this build does not
//! implement yet. On surface `probe`, `click(n)` runs case `n` and answers `replace` with one text
//! node (id 1) whose content reports the results, joined with ` | `:
//!
//! 1. `add-image` of a 4 × 4 PNG, then of an SVG: the two handles;
//! 2. `add-image` of a 1 KiB PNG whose header claims 65535 × 65535 (the bomb), of 1 MiB + 1
//!    bytes, of an SVG declaring entities, and of bytes that are no image: four error codes;
//! 3. `add-image` 34 times: `ok*32,limit-exceeded*2`; then `remove-image` of the first handle and
//!    one more `add-image`: its handle;
//! 4. `push-update`s with `image` nodes (design.md §12, the checks at apply): to `probe` an asset,
//!    a `set-props` and an `insert` (ok), generated handles 0 and 99 that `add-image` never
//!    returned, empty alt text, a 4096 px box and the asset path `../x.png` (invalid-argument),
//!    and the asset to the status item `status` and the modal `modal` (invalid-argument); then
//!    `add-image`, a push of that handle, `remove-image`, and the push again: all their results;
//! 5. answers with an `image` node instead of the report, on whatever surface it came from;
//! 6. calls `permissions.query`, which this build links as a trap;
//! 7. (reserved)
//! 8. `push-update`s with `canvas` nodes (design.md §13): a valid one to `probe` (ok), one with a
//!    NaN coordinate, one without a description, one of 16385 ops, and the valid one to `status`
//!    and to `modal`: all their results;
//! 9. answers with a clickable `canvas` node (id 1) instead of the report.
//!
//! A `canvas-click` is answered with a text node (id 1): `canvas-click <canvas> <x> <y>`.
//!
//! `mount` answers a text node on `probe` and an `image` node anywhere else; `list-rows` answers
//! one row holding an `image` node.
//!
//! `network:fetch` is driven by `input-changed` on `probe`, whose text is a command; the reply
//! reports each result (an id or an error code), joined with ` | `:
//! - `fetch <url>...`: one GET per URL;
//! - `fetch-trap <url>`: a GET, then a trap, so the request is dropped with the call;
//! - `cancel <id>`: `cancel-fetch`, reported as `done`.
//!
//! `handle-io` pushes the outcome to `probe` as one text node (id 1): `fetch-done <id> <status>
//! <body>` or `fetch-done <id> err <error>`.
//!
//! `process:spawn`, by the same `input-changed` commands:
//! - `spawn [stdin] [env NAME=VALUE]... -- <program> [<arg>...]`: its id or error code (`_` in an
//!   argument is a space);
//! - `write <id> <text>` (with a newline), `close <id>`, `kill <id>`: `done` or the error code.
//!
//! Outcomes: `output <id> <stdout|stderr> <text>`, `exited <id> <code|none> <killed>`,
//! `spawn-failed <id> <denied|not-found|limit-exceeded|io>`.
//!
//! `workspace:edit`, by the same commands:
//! - `edit [label=<label>] [expect=<version>] [create] <target>@<l>.<c>-<l>.<c>=<text>...`: one
//!   `apply-edit`; each word is a text edit, consecutive words of one target make one file edit;
//!   the target is a worktree-relative path, `#<n>` a document id; `_` in the text is a space and
//!   an empty text deletes; reports the id or the error code;
//! - `edit-trap <same>`: the edit, then a trap.
//!
//! Outcome: `edit-done <id> ok` or `edit-done <id> err <code>`.
//!
//! `agent:observe`: `observe start` (`done` or the error code), `observe stop` (`done`).
//! Outcomes: `agent snapshot <reviews>`, `agent <opened|updated|resolved> <review> <session>
//! <turn> <hunks> <truncated>`, `agent closed <review>`, `agent turn-ended <turn> <files>`,
//! `agent unreviewed <turn> <files>`, `agent review-mode <on>`, where `<reviews>` is the
//! comma-separated `<review>:<session>:<turn>:<hunks>:<truncated>` of each.

wit_bindgen::generate!({
    path: "../../../ui_ext/wit/since_v0.3.0",
    world: "ui-extension",
});

use fyx::ui::host::{self, HostError, HostErrorCode};
use fyx::ui::images::{self, ImageFormat};
use fyx::ui::network;
use fyx::ui::permissions;
use fyx::ui::process;
use fyx::ui::types::*;
use fyx::ui::{agent_observe, documents, workspace_edit};

#[cfg(target_arch = "wasm32")]
#[unsafe(link_section = "fyx:ui-api-version")]
#[used]
static FYX_UI_API_VERSION: [u8; 6] = [0, 0, 0, 3, 0, 0];

const SURFACE_ID: &str = "probe";

struct Probe;

fn text(id: NodeId, content: &str) -> Node {
    Node {
        id,
        parent: None,
        props: Props::Text(TextProps {
            content: content.into(),
            color: None,
            bold: false,
            monospace: false,
        }),
    }
}

fn image(id: NodeId) -> Node {
    Node {
        id,
        parent: None,
        props: Props::Image(logo()),
    }
}

fn logo() -> ImageProps {
    ImageProps {
        source: ImageSource::Asset("assets/logo.svg".into()),
        alt: "Logo".into(),
        width: Some(32),
        height: None,
        fit: ImageFit::Contain,
        tint: None,
    }
}

fn outcome(result: Result<(), HostError>) -> String {
    match result {
        Ok(()) => "done".into(),
        Err(error) => code(&error).into(),
    }
}

fn push(surface_id: &str, update: Update) -> String {
    match host::push_update(surface_id, &update) {
        Ok(()) => "ok".into(),
        Err(error) => code(&error).into(),
    }
}

fn stroke() -> Stroke {
    Stroke {
        paint: Paint {
            color: CanvasColor::Token(ColorToken::TextAccent),
            opacity: 100,
        },
        width: 1.5,
    }
}

/// A small bar chart with every op kind.
fn chart() -> CanvasProps {
    let fill = Some(Paint {
        color: CanvasColor::Data(DataColor::Series1),
        opacity: 80,
    });
    CanvasProps {
        width: 240,
        height: 120,
        description: "Commits per day".into(),
        ops: vec![
            DrawOp::Rect(RectOp {
                bounds: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 240.0,
                    height: 120.0,
                },
                fill,
                stroke: None,
            }),
            DrawOp::RoundedRect(RoundedRectOp {
                bounds: Rect {
                    x: 10.0,
                    y: 40.0,
                    width: 20.0,
                    height: 80.0,
                },
                radius: 3.0,
                fill,
                stroke: Some(stroke()),
            }),
            DrawOp::Circle(CircleOp {
                center: Point { x: 60.0, y: 60.0 },
                radius: 5.0,
                fill,
                stroke: None,
            }),
            DrawOp::Line(LineOp {
                from: Point { x: 0.0, y: 119.0 },
                to: Point { x: 240.0, y: 119.0 },
                stroke: stroke(),
            }),
            DrawOp::Path(PathOp {
                segments: vec![
                    PathSegment::MoveTo(Point { x: 0.0, y: 90.0 }),
                    PathSegment::LineTo(Point { x: 30.0, y: 40.0 }),
                    PathSegment::QuadTo((Point { x: 45.0, y: 20.0 }, Point { x: 60.0, y: 50.0 })),
                    PathSegment::CubicTo((
                        Point { x: 70.0, y: 60.0 },
                        Point { x: 80.0, y: 30.0 },
                        Point { x: 90.0, y: 40.0 },
                    )),
                    PathSegment::Close,
                ],
                fill: None,
                stroke: Some(stroke()),
            }),
            DrawOp::Text(TextOp {
                at: Point { x: 4.0, y: 14.0 },
                text: "Commits".into(),
                paint: Paint {
                    color: CanvasColor::Token(ColorToken::TextMuted),
                    opacity: 100,
                },
                size: CanvasTextSize::Small,
                anchor: TextAnchor::Start,
                monospace: false,
            }),
        ],
        clickable: true,
        fit: CanvasFit::ScaleDown,
    }
}

fn push_canvas(surface_id: &str, props: CanvasProps) -> String {
    push(
        surface_id,
        Update::Replace(vec![Node {
            id: 2,
            parent: None,
            props: Props::Canvas(props),
        }]),
    )
}

fn push_image(surface_id: &str, props: ImageProps) -> String {
    push(
        surface_id,
        Update::Replace(vec![Node {
            id: 2,
            parent: None,
            props: Props::Image(props),
        }]),
    )
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
    bytes.extend(13u32.to_be_bytes());
    bytes.extend(b"IHDR");
    bytes.extend(width.to_be_bytes());
    bytes.extend(height.to_be_bytes());
    bytes.extend([8, 6, 0, 0, 0, 0, 0, 0, 0]);
    bytes
}

fn code(error: &HostError) -> &'static str {
    match error.code {
        HostErrorCode::UnknownSurface => "unknown-surface",
        HostErrorCode::LimitExceeded => "limit-exceeded",
        HostErrorCode::InvalidArgument => "invalid-argument",
        HostErrorCode::NotPermitted => "not-permitted",
        HostErrorCode::Unavailable => "unavailable",
        HostErrorCode::Denied => "denied",
    }
}

fn add(format: ImageFormat, bytes: &[u8]) -> String {
    match images::add_image(format, bytes) {
        Ok(handle) => handle.to_string(),
        Err(error) => code(&error).into(),
    }
}

fn run(case: NodeId) -> Result<Vec<String>, String> {
    Ok(match case {
        1 => vec![
            add(ImageFormat::Png, &png(4, 4)),
            add(
                ImageFormat::Svg,
                b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>",
            ),
        ],
        2 => {
            let mut bomb = png(65535, 65535);
            bomb.resize(1024, 0);
            let mut big = png(4, 4);
            big.resize((1 << 20) + 1, 0);
            vec![
                add(ImageFormat::Png, &bomb),
                add(ImageFormat::Png, &big),
                add(
                    ImageFormat::Svg,
                    b"<!DOCTYPE x [<!ENTITY a \"aaaa\">]><svg>&a;</svg>",
                ),
                add(ImageFormat::Png, b"not an image at all, just text"),
            ]
        }
        3 => {
            let mut runs: Vec<(String, usize)> = Vec::new();
            let mut first = None;
            for _ in 0..34 {
                let result = match images::add_image(ImageFormat::Png, &png(1, 1)) {
                    Ok(handle) => {
                        first.get_or_insert(handle);
                        "ok".to_string()
                    }
                    Err(error) => code(&error).into(),
                };
                match runs.last_mut() {
                    Some((last, count)) if *last == result => *count += 1,
                    _ => runs.push((result, 1)),
                }
            }
            let summary = runs
                .iter()
                .map(|(result, count)| format!("{result}*{count}"))
                .collect::<Vec<_>>()
                .join(",");
            images::remove_image(first.ok_or("no image was added")?);
            vec![summary, add(ImageFormat::Png, &png(1, 1))]
        }
        4 => {
            let mut results = vec![
                push_image(SURFACE_ID, logo()),
                push(
                    SURFACE_ID,
                    Update::Patches(vec![Patch::SetProps((1, Props::Image(logo())))]),
                ),
                push(
                    SURFACE_ID,
                    Update::Patches(vec![Patch::Insert(InsertOp {
                        node: image(3),
                        before: None,
                    })]),
                ),
            ];
            for props in [
                ImageProps {
                    source: ImageSource::Generated(0),
                    ..logo()
                },
                ImageProps {
                    source: ImageSource::Generated(99),
                    ..logo()
                },
                ImageProps {
                    alt: String::new(),
                    ..logo()
                },
                ImageProps {
                    width: Some(4096),
                    ..logo()
                },
                ImageProps {
                    source: ImageSource::Asset("../x.png".into()),
                    ..logo()
                },
            ] {
                results.push(push_image(SURFACE_ID, props));
            }
            results.push(push_image("status", logo()));
            results.push(push_image("modal", logo()));
            let handle = images::add_image(ImageFormat::Png, &png(4, 4))
                .map_err(|error| code(&error).to_string())?;
            let generated = ImageProps {
                source: ImageSource::Generated(handle),
                ..logo()
            };
            results.push(push_image(SURFACE_ID, generated.clone()));
            images::remove_image(handle);
            results.push(push_image(SURFACE_ID, generated));
            results
        }
        8 => {
            let mut results = vec![push_canvas(SURFACE_ID, chart())];
            let mut nan = chart();
            nan.ops.push(DrawOp::Line(LineOp {
                from: Point {
                    x: f32::NAN,
                    y: 0.0,
                },
                to: Point { x: 1.0, y: 1.0 },
                stroke: stroke(),
            }));
            results.push(push_canvas(SURFACE_ID, nan));
            let mut undescribed = chart();
            undescribed.description.clear();
            results.push(push_canvas(SURFACE_ID, undescribed));
            let mut huge = chart();
            huge.ops = vec![huge.ops[0].clone(); 16385];
            results.push(push_canvas(SURFACE_ID, huge));
            results.push(push_canvas("status", chart()));
            results.push(push_canvas("modal", chart()));
            results
        }
        6 => vec![format!(
            "{:?}",
            permissions::query(permissions::T2Capability::NetworkFetch)
        )],
        other => return Err(format!("no case {other}")),
    })
}

impl Guest for Probe {
    fn surfaces() -> Vec<Surface> {
        // std's `HashMap` seeds itself through `wasi:random/insecure-seed`, at a newer WASI patch
        // version than the host's wasmtime-wasi defines: it must resolve, not trap.
        let mut seen = std::collections::HashMap::new();
        seen.insert(SURFACE_ID, ());
        assert_eq!(seen.len(), 1);
        vec![
            Surface {
                id: SURFACE_ID.into(),
                title: "Probe".into(),
                kind: SurfaceKind::Panel(PanelDescriptor {
                    position: DockPosition::Right,
                    icon: Icon::Image,
                    default_size: 300,
                }),
            },
            Surface {
                id: "status".into(),
                title: "Status".into(),
                kind: SurfaceKind::StatusItem(StatusItemDescriptor {
                    side: StatusBarSide::Right,
                    priority: 0,
                }),
            },
            Surface {
                id: "modal".into(),
                title: "Modal".into(),
                kind: SurfaceKind::Modal(ModalDescriptor {
                    width: ModalWidth::Small,
                }),
            },
        ]
    }

    fn mount(surface_id: String) -> Result<Vec<Node>, String> {
        if surface_id == SURFACE_ID {
            Ok(vec![text(1, "probe")])
        } else {
            Ok(vec![image(1)])
        }
    }

    fn handle_event(_surface_id: String, event: UiEvent) -> Result<Update, String> {
        match event {
            UiEvent::InputChanged((_, command)) => {
                let words: Vec<&str> = command.split_whitespace().collect();
                let fetch = |url: &str| match network::fetch(&network::HttpRequest {
                    method: network::HttpMethod::Get,
                    url: url.into(),
                    headers: Vec::new(),
                    body: None,
                    timeout_ms: None,
                }) {
                    Ok(id) => id.to_string(),
                    Err(error) => code(&error).to_string(),
                };
                let report = match words.as_slice() {
                    ["fetch", urls @ ..] => urls.iter().map(|url| fetch(url)).collect(),
                    ["fetch-trap", url] => {
                        fetch(url);
                        panic!("trap after a fetch")
                    }
                    ["cancel", id] => {
                        network::cancel_fetch(id.parse().map_err(|e| format!("{e}"))?);
                        vec!["done".to_string()]
                    }
                    ["spawn", rest @ ..] => {
                        let split = rest
                            .iter()
                            .position(|word| *word == "--")
                            .ok_or("spawn needs --")?;
                        let (options, program) = rest.split_at(split);
                        let program = &program[1..];
                        let mut env = Vec::new();
                        let mut stdin = false;
                        let mut options = options.iter();
                        while let Some(option) = options.next() {
                            match *option {
                                "stdin" => stdin = true,
                                "env" => {
                                    let pair = options.next().ok_or("env needs NAME=VALUE")?;
                                    let (name, value) =
                                        pair.split_once('=').ok_or("env needs NAME=VALUE")?;
                                    env.push((name.to_string(), value.to_string()));
                                }
                                other => return Err(format!("no spawn option {other}")),
                            }
                        }
                        let (name, args) = program.split_first().ok_or("spawn needs a program")?;
                        vec![match process::spawn(&process::SpawnRequest {
                            program: name.to_string(),
                            args: args.iter().map(|arg| arg.replace('_', " ")).collect(),
                            cwd: None,
                            env,
                            stdin,
                        }) {
                            Ok(id) => id.to_string(),
                            Err(error) => code(&error).to_string(),
                        }]
                    }
                    ["write", id, text] => vec![outcome(process::write_stdin(
                        id.parse().map_err(|e| format!("{e}"))?,
                        format!("{text}\n").as_bytes(),
                    ))],
                    ["close", id] => vec![outcome(process::close_stdin(
                        id.parse().map_err(|e| format!("{e}"))?,
                    ))],
                    ["kill", id] => vec![outcome(process::kill(
                        id.parse().map_err(|e| format!("{e}"))?,
                    ))],
                    ["edit", rest @ ..] => vec![apply_edit(rest)?],
                    ["edit-trap", rest @ ..] => {
                        apply_edit(rest)?;
                        panic!("trap after an edit")
                    }
                    ["observe", "start"] => vec![outcome(agent_observe::start())],
                    ["observe", "stop"] => {
                        agent_observe::stop();
                        vec!["done".to_string()]
                    }
                    _ => return Err(format!("no command {command:?}")),
                };
                Ok(Update::Replace(vec![text(1, &report.join(" | "))]))
            }
            UiEvent::Click(5) => Ok(Update::Replace(vec![image(1)])),
            UiEvent::Click(9) => Ok(Update::Replace(vec![Node {
                id: 1,
                parent: None,
                props: Props::Canvas(chart()),
            }])),
            UiEvent::CanvasClick(click) => Ok(Update::Replace(vec![text(
                1,
                &format!("canvas-click {} {} {}", click.canvas, click.x, click.y),
            )])),
            UiEvent::Click(case) => {
                let report = run(case)?.join(" | ");
                Ok(Update::Replace(vec![text(1, &report)]))
            }
            _ => Ok(Update::Unchanged),
        }
    }

    fn list_rows(_surface_id: String, _list_node: NodeId, start: u32, _count: u32) -> Vec<ListRow> {
        vec![ListRow {
            index: start,
            indent: 0,
            nodes: vec![image(1)],
        }]
    }

    fn tick(_surface_id: String) -> Result<Update, String> {
        Ok(Update::Unchanged)
    }

    fn handle_io(event: fyx::ui::io::IoEvent) -> Result<(), String> {
        let report = match event {
            fyx::ui::io::IoEvent::FetchDone((id, Ok(response))) => format!(
                "fetch-done {id} {} {}",
                response.status,
                String::from_utf8_lossy(&response.body)
            ),
            fyx::ui::io::IoEvent::FetchDone((id, Err(error))) => {
                let error = match error {
                    network::FetchError::Denied => "denied".to_string(),
                    network::FetchError::HostNotAllowed(host) => format!("host-not-allowed {host}"),
                    network::FetchError::RedirectNotAllowed(host) => {
                        format!("redirect-not-allowed {host}")
                    }
                    network::FetchError::Dns(_) => "dns".into(),
                    network::FetchError::Connect(_) => "connect".into(),
                    network::FetchError::Tls(_) => "tls".into(),
                    network::FetchError::Timeout => "timeout".into(),
                    network::FetchError::TooLarge => "too-large".into(),
                    network::FetchError::Cancelled => "cancelled".into(),
                    network::FetchError::Other(_) => "other".into(),
                };
                format!("fetch-done {id} err {error}")
            }
            fyx::ui::io::IoEvent::ProcessOutput(output) => format!(
                "output {} {} {}",
                output.process,
                match output.stream {
                    process::OutputStream::Stdout => "stdout",
                    process::OutputStream::Stderr => "stderr",
                },
                String::from_utf8_lossy(&output.data)
            ),
            fyx::ui::io::IoEvent::ProcessExited(exit) => format!(
                "exited {} {} {}",
                exit.process,
                exit.code
                    .map_or("none".to_string(), |code| code.to_string()),
                exit.killed
            ),
            fyx::ui::io::IoEvent::SpawnFailed((id, error)) => format!(
                "spawn-failed {id} {}",
                match error {
                    process::SpawnError::Denied => "denied",
                    process::SpawnError::NotFound(_) => "not-found",
                    process::SpawnError::LimitExceeded => "limit-exceeded",
                    process::SpawnError::Io(_) => "io",
                }
            ),
            fyx::ui::io::IoEvent::EditDone((id, Ok(()))) => format!("edit-done {id} ok"),
            fyx::ui::io::IoEvent::EditDone((id, Err(error))) => format!(
                "edit-done {id} err {}",
                match error {
                    workspace_edit::EditError::Denied => "denied",
                    workspace_edit::EditError::Conflict(_) => "conflict",
                    workspace_edit::EditError::NotAllowed(_) => "not-allowed",
                    workspace_edit::EditError::NotFound(_) => "not-found",
                    workspace_edit::EditError::Invalid(_) => "invalid",
                    workspace_edit::EditError::Cancelled => "cancelled",
                }
            ),
            fyx::ui::io::IoEvent::Agent(event) => format!("agent {}", agent_report(&event)),
            fyx::ui::io::IoEvent::PermissionChanged((capability, state)) => {
                format!("permission-changed {capability:?} {state:?}")
            }
            _ => return Err("unexpected io event".into()),
        };
        host::push_update(SURFACE_ID, &Update::Replace(vec![text(1, &report)]))
            .map_err(|error| code(&error).to_string())
    }

    fn handle_command(
        _command: String,
        _argument: Option<String>,
        _context: fyx::ui::requests::CommandContext,
    ) -> Result<(), String> {
        Err("unsupported".into())
    }

    fn provide_decorations(
        _request: fyx::ui::requests::DecorationRequest,
    ) -> Result<Vec<fyx::ui::decorations::Decoration>, String> {
        Err("unsupported".into())
    }

    fn provide_hover(
        _request: fyx::ui::requests::HoverRequest,
    ) -> Result<fyx::ui::requests::HoverReply, String> {
        Err("unsupported".into())
    }

    fn provide_code_lenses(
        _request: fyx::ui::requests::CodeLensRequest,
    ) -> Result<Vec<fyx::ui::requests::CodeLens>, String> {
        Err("unsupported".into())
    }

    fn provide_virtual_document(
        _path: String,
    ) -> Result<fyx::ui::requests::VirtualDocument, String> {
        Err("unsupported".into())
    }

    fn picker_source_event(
        _source: String,
        _session: u64,
        _event: fyx::ui::requests::PickerSourceEvent,
    ) -> Result<(), String> {
        Err("unsupported".into())
    }
}

fn position(text: &str) -> Result<documents::Position, String> {
    let (line, column) = text
        .split_once('.')
        .ok_or("a position is <line>.<column>")?;
    Ok(documents::Position {
        line: line.parse().map_err(|e| format!("{e}"))?,
        column: column.parse().map_err(|e| format!("{e}"))?,
    })
}

/// The `edit` command: see the module doc.
fn apply_edit(words: &[&str]) -> Result<String, String> {
    let mut label = "Probe edit".to_string();
    let mut expected_version = None;
    let mut create = false;
    let mut files: Vec<(String, workspace_edit::FileEdit)> = Vec::new();
    for word in words {
        if let Some(value) = word.strip_prefix("label=") {
            label = value.replace('_', " ");
            continue;
        }
        if let Some(value) = word.strip_prefix("expect=") {
            expected_version = Some(value.parse().map_err(|e| format!("{e}"))?);
            continue;
        }
        if *word == "create" {
            create = true;
            continue;
        }
        let (target, edit) = word
            .split_once('@')
            .ok_or("an edit is <target>@<range>=<text>")?;
        let (range, text) = edit
            .split_once('=')
            .ok_or("an edit is <target>@<range>=<text>")?;
        let (start, end) = range.split_once('-').ok_or("a range is <start>-<end>")?;
        let text_edit = workspace_edit::TextEdit {
            range: documents::TextRange {
                start: position(start)?,
                end: position(end)?,
            },
            new_text: text.replace('_', " "),
        };
        match files.last_mut() {
            Some((last, file)) if last == target => file.edits.push(text_edit),
            _ => files.push((
                target.to_string(),
                workspace_edit::FileEdit {
                    target: match target.strip_prefix('#') {
                        Some(id) => workspace_edit::EditTarget::Document(
                            id.parse().map_err(|e| format!("{e}"))?,
                        ),
                        None => workspace_edit::EditTarget::File(documents::FileRef {
                            worktree: None,
                            path: target.to_string(),
                        }),
                    },
                    expected_version: None,
                    create: false,
                    edits: vec![text_edit],
                },
            )),
        }
    }
    let files = files
        .into_iter()
        .map(|(_, mut file)| {
            file.expected_version = expected_version;
            file.create = create;
            file
        })
        .collect();
    Ok(
        match workspace_edit::apply_edit(&workspace_edit::WorkspaceEdit { label, files }) {
            Ok(id) => id.to_string(),
            Err(error) => code(&error).to_string(),
        },
    )
}

fn review_report(info: &agent_observe::ReviewInfo) -> String {
    format!(
        "{}:{}:{}:{}:{}",
        info.review,
        info.session,
        info.turn,
        info.hunks.len(),
        info.truncated
    )
}

fn agent_report(event: &agent_observe::AgentEvent) -> String {
    use agent_observe::AgentEvent;
    match event {
        AgentEvent::Snapshot(reviews) => format!(
            "snapshot {}",
            reviews
                .iter()
                .map(review_report)
                .collect::<Vec<_>>()
                .join(",")
        ),
        AgentEvent::Opened(info) => format!("opened {}", review_report(info).replace(':', " ")),
        AgentEvent::Updated(info) => format!("updated {}", review_report(info).replace(':', " ")),
        AgentEvent::Resolved(info) => {
            format!("resolved {}", review_report(info).replace(':', " "))
        }
        AgentEvent::Closed(review) => format!("closed {review}"),
        AgentEvent::TurnEnded((turn, files)) => format!("turn-ended {turn} {}", files.len()),
        AgentEvent::UnreviewedChanges((turn, files)) => {
            format!("unreviewed {turn} {}", files.len())
        }
        AgentEvent::ReviewMode(on) => format!("review-mode {on}"),
    }
}

export!(Probe);
