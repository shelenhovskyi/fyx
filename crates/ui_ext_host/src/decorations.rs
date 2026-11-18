//! Buffer decorations (layer 4, stretch): a retained `fyx:ui` tree drawn as an editor block above
//! or below a buffer line.
//!
//! `fyx:ui@0.1.0` has no decoration API, so for the sprint this is a host-side Rust API
//! ([`Decorations`]) plus a dev-only data source ([`DEV_DECORATIONS_ENV`]). The WIT shape proposed
//! for 0.2 is in `tasks/wit-0.2.md`.
//!
//! - A decoration names a file and a line. The first time a buffer for that file shows up in an
//!   editor, the line becomes a CRDT anchor in that buffer, kept for the decoration's lifetime:
//!   edits above the line move the block with it, and every editor on that buffer (splits,
//!   multibuffers) places the block at the same logical line, however much was edited before it
//!   opened.
//! - The tree lives in a [`SurfaceState`] and is drawn by a [`SurfaceView`], the same renderer as
//!   panels, so it uses theme tokens only and never calls an extension while rendering.
//! - Editors are found with `observe_new::<Editor>` and use the editor's public block API, so no
//!   upstream edit is needed.

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
};

use anyhow::{Context as _, Result};
use collections::{HashMap, HashSet};
use editor::{
    Editor, EditorEvent,
    display_map::{BlockPlacement, BlockProperties, BlockStyle, CustomBlockId},
};
use futures::{FutureExt as _, future::LocalBoxFuture};
use gpui::{
    App, AppContext as _, Context, Entity, EntityId, Global, InteractiveElement as _,
    IntoElement as _, ParentElement as _, Styled as _, Window, div,
};
use language::{Bias, BufferId, Point};
use serde::Deserialize;
use ui_ext::TreeLimits;
use ui_ext::v0_2::{
    Align, ColorToken, FlexDirection, FlexProps, Node, NodeId, Props, StatusBarSide,
    StatusItemDescriptor, Surface, SurfaceKind, TextProps, UiEvent, Update, tree::Tree,
};

use crate::{
    SurfaceState, SurfaceView,
    backend::{Reply, UiBackend},
};

/// Dev-only data source for the sprint: the path of a JSON file (comments allowed) holding a list
/// of decorations, read once at startup. Each entry is
/// `{ "path": "...", "line": 3, "text": "..." }` or, instead of `text`, a flat `nodes` tree in the
/// `fyx:ui` serde form; optional `"placement": "above" | "below"` (default below) and `"lines"`
/// (block height in editor lines; estimated from the tree when absent). `line` is 1-based, as in
/// the gutter. `path` is absolute, or a path suffix of the worktree-qualified path
/// (`"scratch/notes.txt"` or just `"notes.txt"`).
pub const DEV_DECORATIONS_ENV: &str = "FYX_UI_DEV_DECORATIONS";

/// Blocks taller than this are clamped; the tree scrolls inside the block.
const MAX_BLOCK_LINES: u32 = 40;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DecorationPlacement {
    Above,
    #[default]
    Below,
}

/// Where a decoration goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecorationTarget {
    /// Absolute, or a suffix of the worktree-qualified path (compared by components).
    pub path: PathBuf,
    /// Zero-based buffer row. Clipped to the last row of the buffer.
    pub row: u32,
    pub placement: DecorationPlacement,
    /// Block height in editor lines; `None` estimates it from the tree.
    pub lines: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DecorationId(u64);

struct Decoration {
    id: DecorationId,
    target: DecorationTarget,
    state: Entity<SurfaceState>,
    /// Set for trees given by the host ([`Decorations::add_tree`]): updates are written back into
    /// it, so a remount shows the latest tree instead of the first.
    static_backend: Option<Rc<StaticBackend>>,
    /// The line as a CRDT anchor, per buffer it was found in.
    anchors: HashMap<BufferId, language::Anchor>,
}

/// One block a decoration has in one editor.
struct Placed {
    block: CustomBlockId,
    lines: u32,
    _view: Entity<SurfaceView>,
}

/// Every decoration, and the blocks each editor has for them.
#[derive(Default)]
pub struct Decorations {
    decorations: Vec<Decoration>,
    next_id: u64,
    editors: HashMap<EntityId, HashMap<(DecorationId, BufferId), Placed>>,
}

struct GlobalDecorations(Entity<Decorations>);

impl Global for GlobalDecorations {}

impl Decorations {
    pub fn global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GlobalDecorations>()
            .map(|global| global.0.clone())
    }

    /// A decoration driven by an extension surface: the tree comes from its `mount`, clicks go to
    /// its `handle-event`, and pushes for the surface go to [`Self::update`].
    pub fn add(
        &mut self,
        target: DecorationTarget,
        backend: Rc<dyn UiBackend>,
        surface: Surface,
        cx: &mut Context<Self>,
    ) -> DecorationId {
        self.insert(target, backend, surface, None, cx)
    }

    /// A decoration showing a host-supplied tree. Events are dropped.
    pub fn add_tree(
        &mut self,
        target: DecorationTarget,
        nodes: Vec<Node>,
        cx: &mut Context<Self>,
    ) -> Result<DecorationId> {
        Tree::from_nodes(nodes.clone(), TreeLimits::default())?;
        let id = DecorationId(self.next_id);
        let backend = Rc::new(StaticBackend {
            nodes: RefCell::new(nodes),
        });
        // 0.1 has no decoration surface kind; `SurfaceState` never reads `kind`.
        let surface = Surface {
            id: format!("decoration-{}", id.0),
            title: "Decoration".into(),
            kind: SurfaceKind::StatusItem(StatusItemDescriptor {
                side: StatusBarSide::Left,
                priority: 0,
            }),
        };
        Ok(self.insert(target, backend.clone(), surface, Some(backend), cx))
    }

    fn insert(
        &mut self,
        target: DecorationTarget,
        backend: Rc<dyn UiBackend>,
        surface: Surface,
        static_backend: Option<Rc<StaticBackend>>,
        cx: &mut Context<Self>,
    ) -> DecorationId {
        let id = DecorationId(self.next_id);
        self.next_id += 1;
        let state = cx.new(|cx| SurfaceState::new(backend, surface, None, cx));
        // A mounted tree can change the estimated height.
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        self.decorations.push(Decoration {
            id,
            target,
            state,
            static_backend,
            anchors: HashMap::default(),
        });
        cx.notify();
        id
    }

    /// Applies an update to a decoration's tree. Returns whether it applied.
    pub fn update(&mut self, id: DecorationId, update: Update, cx: &mut Context<Self>) -> bool {
        let Some(decoration) = self.decoration(id) else {
            return false;
        };
        let state = decoration.state.clone();
        let static_backend = decoration.static_backend.clone();
        state.update(cx, |state, cx| {
            let applied = state.apply_update(update, cx);
            if applied
                && let Some(backend) = static_backend
                && let Some(tree) = state.tree()
            {
                *backend.nodes.borrow_mut() = tree.to_nodes();
            }
            applied
        })
    }

    pub fn remove(&mut self, id: DecorationId, cx: &mut Context<Self>) {
        let len = self.decorations.len();
        self.decorations.retain(|decoration| decoration.id != id);
        if self.decorations.len() != len {
            cx.notify();
        }
    }

    pub fn state(&self, id: DecorationId) -> Option<&Entity<SurfaceState>> {
        self.decoration(id).map(|decoration| &decoration.state)
    }

    /// The blocks a decoration has in an editor (one per matching buffer).
    pub fn blocks_in(&self, editor: EntityId, id: DecorationId) -> Vec<CustomBlockId> {
        let mut blocks: Vec<_> = self
            .editors
            .get(&editor)
            .into_iter()
            .flatten()
            .filter(|((decoration, _), _)| *decoration == id)
            .map(|(_, placed)| placed.block)
            .collect();
        blocks.sort();
        blocks
    }

    fn decoration(&self, id: DecorationId) -> Option<&Decoration> {
        self.decorations
            .iter()
            .find(|decoration| decoration.id == id)
    }
}

pub(crate) fn init(cx: &mut App) {
    let decorations = cx.new(|_| Decorations::default());
    cx.set_global(GlobalDecorations(decorations.clone()));

    cx.observe_new(move |editor: &mut Editor, window, cx| {
        let Some(window) = window else {
            return;
        };
        if !editor.mode().is_full() {
            return;
        }
        cx.observe_in(&decorations, window, |editor, _, window, cx| {
            sync_editor(editor, window, cx)
        })
        .detach();
        cx.subscribe_in(
            &cx.entity(),
            window,
            |editor, _, event: &EditorEvent, window, cx| {
                if matches!(
                    event,
                    EditorEvent::BufferRangesUpdated { .. }
                        | EditorEvent::BuffersRemoved { .. }
                        | EditorEvent::TitleChanged
                        | EditorEvent::Saved
                ) {
                    sync_editor(editor, window, cx);
                }
            },
        )
        .detach();
        let decorations = decorations.downgrade();
        let editor_id = cx.entity_id();
        cx.on_release(move |_, cx| {
            decorations
                .update(cx, |decorations, _| decorations.editors.remove(&editor_id))
                .ok();
        })
        .detach();
        sync_editor(editor, window, cx);
    })
    .detach();

    if let Some(path) = std::env::var_os(DEV_DECORATIONS_ENV) {
        load_dev_decorations(PathBuf::from(path), cx);
    }
}

/// Brings an editor's blocks in line with the decorations: one block per decoration and matching
/// buffer, at the decoration's anchor in that buffer.
fn sync_editor(editor: &mut Editor, window: &mut Window, cx: &mut Context<Editor>) {
    let Some(decorations) = Decorations::global(cx) else {
        return;
    };
    let editor_id = cx.entity_id();
    let multibuffer = editor.buffer().read(cx);
    let buffers: Vec<_> = multibuffer.all_buffers_iter().collect();
    let snapshot = multibuffer.snapshot(cx);

    // Which decorations belong in this editor, anchoring lines in buffers seen for the first time.
    let mut wanted = Vec::new();
    let mut new_anchors = Vec::new();
    {
        let decorations = decorations.read(cx);
        for buffer in &buffers {
            let buffer = buffer.read(cx);
            let Some(file) = buffer.file() else {
                continue;
            };
            let full_path = file.full_path(cx);
            let abs_path = file.as_local().map(|file| file.abs_path(cx));
            let buffer_id = buffer.remote_id();
            for decoration in &decorations.decorations {
                if !path_matches(&decoration.target.path, &full_path, abs_path.as_deref()) {
                    continue;
                }
                let anchor = match decoration.anchors.get(&buffer_id) {
                    Some(anchor) => *anchor,
                    None => {
                        let point =
                            buffer.clip_point(Point::new(decoration.target.row, 0), Bias::Left);
                        // Biased right, so text inserted at the start of the line (a line typed
                        // above it) pushes the anchor down with the line.
                        let anchor = buffer.anchor_after(point);
                        new_anchors.push((decoration.id, buffer_id, anchor));
                        anchor
                    }
                };
                let Some(anchor) = snapshot.anchor_in_excerpt(anchor) else {
                    continue;
                };
                let lines = decoration
                    .target
                    .lines
                    .unwrap_or_else(|| decoration.state.read(cx).tree().map_or(1, estimate_lines))
                    .clamp(1, MAX_BLOCK_LINES);
                wanted.push((
                    (decoration.id, buffer_id),
                    anchor,
                    decoration.target.placement,
                    lines,
                    decoration.state.clone(),
                ));
            }
        }
    }

    let mut placed = decorations.update(cx, |decorations, _| {
        for (id, buffer_id, anchor) in new_anchors {
            if let Some(decoration) = decorations
                .decorations
                .iter_mut()
                .find(|decoration| decoration.id == id)
            {
                decoration.anchors.insert(buffer_id, anchor);
            }
        }
        decorations.editors.remove(&editor_id).unwrap_or_default()
    });

    let wanted_keys: HashSet<_> = wanted.iter().map(|(key, ..)| *key).collect();
    let stale: HashSet<CustomBlockId> = placed
        .extract_if(|key, _| !wanted_keys.contains(key))
        .map(|(_, placed)| placed.block)
        .collect();
    if !stale.is_empty() {
        editor.remove_blocks(stale, None, cx);
    }

    let mut resized = HashMap::default();
    let mut new_blocks = Vec::new();
    for (key, anchor, placement, lines, state) in wanted {
        if let Some(existing) = placed.get_mut(&key) {
            if existing.lines != lines {
                existing.lines = lines;
                resized.insert(existing.block, lines);
            }
            continue;
        }
        let view = cx.new(|cx| SurfaceView::new(state, false, window, cx));
        let placement = match placement {
            DecorationPlacement::Above => BlockPlacement::Above(anchor),
            DecorationPlacement::Below => BlockPlacement::Below(anchor),
        };
        let block_view = view.clone();
        let properties = BlockProperties {
            placement,
            height: Some(lines),
            style: BlockStyle::Sticky,
            render: Arc::new(move |cx| {
                div()
                    .block_mouse_except_scroll()
                    .flex()
                    .w_full()
                    .h(cx.height as f32 * cx.line_height)
                    .pl(cx.margins.gutter.full_width())
                    .child(block_view.clone())
                    .into_any_element()
            }),
            priority: 0,
        };
        new_blocks.push((key, lines, view, properties));
    }
    if !resized.is_empty() {
        editor.resize_blocks(resized, None, cx);
    }
    if !new_blocks.is_empty() {
        let (meta, properties): (Vec<_>, Vec<_>) = new_blocks
            .into_iter()
            .map(|(key, lines, view, properties)| ((key, lines, view), properties))
            .unzip();
        let ids = editor.insert_blocks(properties, None, cx);
        for ((key, lines, view), block) in meta.into_iter().zip(ids) {
            placed.insert(
                key,
                Placed {
                    block,
                    lines,
                    _view: view,
                },
            );
        }
    }

    decorations.update(cx, |decorations, _| {
        if !placed.is_empty() {
            decorations.editors.insert(editor_id, placed);
        }
    });
}

/// `wanted` is absolute (compared with the file's absolute path) or a suffix of the
/// worktree-qualified path, by whole components.
pub(crate) fn path_matches(wanted: &Path, full_path: &Path, abs_path: Option<&Path>) -> bool {
    if wanted.is_absolute() {
        abs_path == Some(wanted)
    } else {
        !wanted.as_os_str().is_empty() && full_path.ends_with(wanted)
    }
}

/// A rough height in editor lines: columns add their children, rows take the tallest. Padding
/// and gaps are ignored; a tree taller than its block scrolls inside it.
pub(crate) fn estimate_lines(tree: &Tree) -> u32 {
    fn lines_of(tree: &Tree, id: NodeId) -> u32 {
        let Some(node) = tree.get(id) else {
            return 0;
        };
        let children = node.children().iter().map(|&child| lines_of(tree, child));
        match node.props() {
            Props::Flex(props) => match props.direction {
                FlexDirection::Column => children.sum(),
                FlexDirection::Row => children.max().unwrap_or(0),
            },
            Props::Text(props) => props.content.lines().count().max(1) as u32,
            Props::Markdown(source) => source.lines().count().max(1) as u32,
            Props::DiffView(props) => props.new_text.lines().count().max(1) as u32 + 1,
            Props::List(props) => props.row_count.clamp(1, 3) + 1,
            Props::Table(props) => props.row_count.clamp(1, 3) + 1,
            Props::Button(_) | Props::Input(_) | Props::Icon(_) => 1,
        }
    }
    tree.roots()
        .iter()
        .map(|&root| lines_of(tree, root))
        .sum::<u32>()
        .max(1)
}

/// Serves a host-supplied tree; events change nothing.
struct StaticBackend {
    nodes: RefCell<Vec<Node>>,
}

impl UiBackend for StaticBackend {
    fn extension_id(&self) -> &str {
        "fyx-decorations"
    }

    fn mount(&self, _: &str) -> LocalBoxFuture<'static, Result<Reply<Tree>>> {
        let tree = Tree::from_nodes(self.nodes.borrow().clone(), TreeLimits::default())
            .map_err(anyhow::Error::from);
        async move {
            tree.map(|value| Reply {
                pushed: Vec::new(),
                value,
            })
        }
        .boxed_local()
    }

    fn handle_event(&self, _: &str, _: UiEvent) -> LocalBoxFuture<'static, Result<Reply<Update>>> {
        async move {
            Ok(Reply {
                pushed: Vec::new(),
                value: Update::Unchanged,
            })
        }
        .boxed_local()
    }

    fn list_rows(
        &self,
        _: &str,
        _: NodeId,
        _: u32,
        _: u32,
    ) -> LocalBoxFuture<'static, Result<Reply<Vec<(u32, Tree)>>>> {
        async move {
            Ok(Reply {
                pushed: Vec::new(),
                value: Vec::new(),
            })
        }
        .boxed_local()
    }
}

/// One entry of the [`DEV_DECORATIONS_ENV`] file.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DevDecoration {
    path: PathBuf,
    /// 1-based.
    line: u32,
    #[serde(default)]
    placement: DecorationPlacement,
    #[serde(default)]
    lines: Option<u32>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    nodes: Option<Vec<Node>>,
}

impl DevDecoration {
    fn into_parts(self) -> Result<(DecorationTarget, Vec<Node>)> {
        let nodes = match (self.text, self.nodes) {
            (Some(text), None) => text_tree(text),
            (None, Some(nodes)) => nodes,
            _ => anyhow::bail!("give exactly one of `text` and `nodes`"),
        };
        let target = DecorationTarget {
            path: self.path,
            row: self.line.saturating_sub(1),
            placement: self.placement,
            lines: self.lines,
        };
        Ok((target, nodes))
    }
}

/// The tree for a `text` entry: one accent-coloured line on the elevated surface.
pub fn text_tree(text: impl Into<String>) -> Vec<Node> {
    vec![
        Node {
            id: 1,
            parent: None,
            props: Props::Flex(FlexProps {
                direction: FlexDirection::Row,
                gap: 4,
                padding: 2,
                grow: true,
                align: Align::Center,
                background: Some(ColorToken::ElevatedSurface),
            }),
        },
        Node {
            id: 2,
            parent: Some(1),
            props: Props::Text(TextProps {
                content: text.into(),
                color: Some(ColorToken::TextAccent),
                bold: false,
                monospace: false,
            }),
        },
    ]
}

pub(crate) fn parse_dev_decorations(content: &str) -> Result<Vec<(DecorationTarget, Vec<Node>)>> {
    let entries: Vec<DevDecoration> = settings::parse_json_with_comments(content)?;
    entries
        .into_iter()
        .enumerate()
        .map(|(index, entry)| {
            entry
                .into_parts()
                .with_context(|| format!("decoration {index}"))
        })
        .collect()
}

fn load_dev_decorations(path: PathBuf, cx: &mut App) {
    let read = cx.background_spawn({
        let path = path.clone();
        async move { std::fs::read_to_string(&path) }
    });
    cx.spawn(async move |cx| {
        let result = read
            .await
            .with_context(|| format!("reading {}", path.display()))
            .and_then(|content| parse_dev_decorations(&content));
        let entries = match result {
            Ok(entries) => entries,
            Err(error) => {
                log::error!("fyx:ui: {DEV_DECORATIONS_ENV}: {error:#}");
                return;
            }
        };
        cx.update(|cx| {
            let Some(decorations) = Decorations::global(cx) else {
                return;
            };
            decorations.update(cx, |decorations, cx| {
                let count = entries.len();
                for (index, (target, nodes)) in entries.into_iter().enumerate() {
                    if let Err(error) = decorations.add_tree(target, nodes, cx) {
                        log::error!("fyx:ui: {DEV_DECORATIONS_ENV}: decoration {index}: {error:#}");
                    }
                }
                log::info!("fyx:ui: loaded {count} dev decoration(s)");
            });
        });
    })
    .detach();
}
