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
//! 6. calls `permissions.query`, which this build links as a trap.
//!
//! `mount` answers a text node on `probe` and an `image` node anywhere else; `list-rows` answers
//! one row holding an `image` node.

wit_bindgen::generate!({
    path: "../../../ui_ext/wit/since_v0.3.0",
    world: "ui-extension",
});

use fyx::ui::host::{self, HostError, HostErrorCode};
use fyx::ui::images::{self, ImageFormat};
use fyx::ui::permissions;
use fyx::ui::types::*;

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

fn push(surface_id: &str, update: Update) -> String {
    match host::push_update(surface_id, &update) {
        Ok(()) => "ok".into(),
        Err(error) => code(&error).into(),
    }
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
            UiEvent::Click(5) => Ok(Update::Replace(vec![image(1)])),
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

    fn handle_io(_event: fyx::ui::io::IoEvent) -> Result<(), String> {
        Err("unsupported".into())
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

export!(Probe);
