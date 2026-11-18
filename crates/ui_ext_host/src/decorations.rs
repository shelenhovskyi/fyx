//! Buffer decorations (layer 4): a retained `fyx:ui` tree drawn as an editor block above or below
//! a buffer line.
//!
//! Three sources feed [`Decorations`]: extensions through `fyx:ui@0.2.0`'s `decorations` interface
//! ([`Decorations::place_extension`], design.md §L), host code ([`Decorations::add`],
//! [`Decorations::add_tree`]) and a dev-only data source ([`DEV_DECORATIONS_ENV`]).
//!
//! - A decoration names a file and a line. The first time a buffer for that file shows up in an
//!   editor, the line becomes a CRDT anchor in that buffer, kept for the decoration's lifetime:
//!   edits above the line move the block with it, and every editor on that buffer (splits,
//!   multibuffers) places the block at the same logical line, however much was edited before it
//!   opened.
//! - The tree lives in a [`SurfaceState`] and is drawn by a [`SurfaceView`], the same renderer as
//!   panels, so it uses theme tokens only and never calls an extension while rendering.
//! - Extension decorations live in a namespace per extension (`"<extension>/<id>"`), at most
//!   [`ui_ext::MAX_DECORATIONS`] each. Their paths are worktree-relative and matched exactly, so an
//!   extension cannot probe for files outside the open worktrees. The block's events and rows go
//!   to the extension as `handle-event(id, ..)` and `list-rows(id, ..)`.
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
use ui_ext::{
    Align, ColorToken, FlexDirection, FlexProps, Node, NodeId, Props, StatusBarSide,
    StatusItemDescriptor, Surface, SurfaceKind, TextProps, Tree, TreeLimits, UiEvent, Update,
};

use crate::{
    Phase, SurfaceState, SurfaceView,
    backend::{Reply, UiBackend},
    surface::SurfaceLink,
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

impl From<ui_ext::v0_2::DecorationPlacement> for DecorationPlacement {
    fn from(placement: ui_ext::v0_2::DecorationPlacement) -> Self {
        match placement {
            ui_ext::v0_2::DecorationPlacement::Above => Self::Above,
            ui_ext::v0_2::DecorationPlacement::Below => Self::Below,
        }
    }
}

/// How a decoration names its file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecorationPath {
    /// Host and dev decorations: absolute (compared with the file's absolute path), or a suffix of
    /// the worktree-qualified path by whole components (`"scratch/notes.txt"`, `"notes.txt"`).
    Host(PathBuf),
    /// Extension decorations (design.md §L): exactly this worktree-relative, `/`-separated path,
    /// in any worktree of any open workspace. Checked with [`ui_ext::check_decoration_path`].
    Worktree(String),
}

impl From<&str> for DecorationPath {
    fn from(path: &str) -> Self {
        Self::Host(path.into())
    }
}

impl From<PathBuf> for DecorationPath {
    fn from(path: PathBuf) -> Self {
        Self::Host(path)
    }
}

/// Where a decoration goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecorationTarget {
    pub path: DecorationPath,
    /// Zero-based buffer row. Clipped to the last row of the buffer.
    pub row: u32,
    pub placement: DecorationPlacement,
    /// Block height in editor lines; `None` estimates it from the tree.
    pub lines: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DecorationId(u64);

/// The extension that placed a decoration, and the id it gave it.
struct Owner {
    extension_id: Arc<str>,
    id: String,
}

struct Decoration {
    id: DecorationId,
    owner: Option<Owner>,
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
        self.insert(target, backend, surface, None, None, None, cx)
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
        Ok(self.insert(
            target,
            backend.clone(),
            surface,
            Some(backend),
            None,
            None,
            cx,
        ))
    }

    /// Applies an extension's `place-decoration(id, location, nodes)` effect (design.md §L).
    ///
    /// The host checked the call (capability, id rule, path rule, caps); the path and the
    /// per-extension cap are checked here again, so no other caller can get around them. A live
    /// id moves: the old decoration, its blocks and its queued events are dropped, and the line is
    /// anchored anew. A tree that does not fit is not shown; the extension is asked for
    /// `mount(id)` instead, and a decoration whose mount fails too is removed with one warning.
    ///
    /// `backend` is the extension's: calls for the decoration use `id` as the surface id. `link`
    /// routes what those calls push for other surfaces back to the registry.
    pub fn place_extension(
        &mut self,
        extension_id: &str,
        id: &str,
        location: &ui_ext::v0_2::DecorationLocation,
        nodes: Vec<Node>,
        backend: Rc<dyn UiBackend>,
        link: Option<Rc<dyn SurfaceLink>>,
        cx: &mut Context<Self>,
    ) -> Result<DecorationId> {
        ui_ext::check_decoration_path(&location.path)?;
        let existing = self.extension_decoration(extension_id, id);
        if existing.is_none() && self.count_of(extension_id) >= ui_ext::MAX_DECORATIONS {
            anyhow::bail!(
                "{extension_id} already has {} decorations",
                ui_ext::MAX_DECORATIONS
            );
        }
        if let Some(existing) = existing {
            self.remove(existing, cx);
        }
        let placed = match Tree::from_nodes(nodes, TreeLimits::default()) {
            Ok(tree) => Some(tree),
            Err(error) => {
                log::warn!(
                    "fyx:ui {extension_id}/{id}: placed tree does not fit ({error}), mounting"
                );
                None
            }
        };
        let backend = Rc::new(PlacedBackend {
            inner: backend,
            placed: RefCell::new(placed),
        });
        // Decorations are not surfaces; `SurfaceState` never reads `kind`.
        let surface = Surface {
            id: id.to_string(),
            title: id.to_string(),
            kind: SurfaceKind::StatusItem(StatusItemDescriptor {
                side: StatusBarSide::Left,
                priority: 0,
            }),
        };
        let target = DecorationTarget {
            path: DecorationPath::Worktree(location.path.clone()),
            row: location.line,
            placement: location.placement.into(),
            lines: location.height_lines.map(u32::from),
        };
        let owner = Owner {
            extension_id: extension_id.into(),
            id: id.to_string(),
        };
        Ok(self.insert(target, backend, surface, None, Some(owner), link, cx))
    }

    /// Applies `remove-decoration(id)`. Returns whether the decoration was live.
    pub fn remove_extension_decoration(
        &mut self,
        extension_id: &str,
        id: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        match self.extension_decoration(extension_id, id) {
            Some(decoration) => {
                self.remove(decoration, cx);
                true
            }
            None => false,
        }
    }

    /// Removes every decoration of an extension: it was disabled, unloaded or is being reloaded.
    /// Returns how many there were.
    pub fn remove_extension(&mut self, extension_id: &str, cx: &mut Context<Self>) -> usize {
        let len = self.decorations.len();
        self.decorations
            .retain(|decoration| !decoration.is_owned_by(extension_id));
        let removed = len - self.decorations.len();
        if removed > 0 {
            cx.notify();
        }
        removed
    }

    /// Applies a `push-update(id, ..)` aimed at a decoration. Returns `false` when the extension
    /// has no live decoration with that id.
    pub fn push(
        &mut self,
        extension_id: &str,
        id: &str,
        update: Update,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(state) = self
            .extension_decoration(extension_id, id)
            .and_then(|decoration| self.state(decoration))
            .cloned()
        else {
            return false;
        };
        state.update(cx, |state, cx| state.apply_pushed_update(update, cx));
        true
    }

    /// The live decoration an extension placed with this id.
    pub fn extension_decoration(&self, extension_id: &str, id: &str) -> Option<DecorationId> {
        self.decorations
            .iter()
            .find(|decoration| {
                decoration
                    .owner
                    .as_ref()
                    .is_some_and(|owner| &*owner.extension_id == extension_id && owner.id == id)
            })
            .map(|decoration| decoration.id)
    }

    /// How many live decorations an extension has.
    pub fn count_of(&self, extension_id: &str) -> usize {
        self.decorations
            .iter()
            .filter(|decoration| decoration.is_owned_by(extension_id))
            .count()
    }

    #[allow(clippy::too_many_arguments)]
    fn insert(
        &mut self,
        target: DecorationTarget,
        backend: Rc<dyn UiBackend>,
        surface: Surface,
        static_backend: Option<Rc<StaticBackend>>,
        owner: Option<Owner>,
        link: Option<Rc<dyn SurfaceLink>>,
        cx: &mut Context<Self>,
    ) -> DecorationId {
        let id = DecorationId(self.next_id);
        self.next_id += 1;
        let state = cx.new(|cx| SurfaceState::new(backend, surface, link, cx));
        let owned = owner.is_some();
        cx.observe(&state, move |this, state, cx| {
            // An extension decoration that cannot be mounted is removed, not shown as an error.
            if owned && let Phase::Failed { message, .. } = state.read(cx).phase() {
                if let Some(owner) = this.decoration(id).and_then(|d| d.owner.as_ref()) {
                    log::warn!(
                        "fyx:ui {}/{}: decoration removed: {message}",
                        owner.extension_id,
                        owner.id
                    );
                }
                this.remove(id, cx);
            }
            // A mounted tree can change the estimated height.
            cx.notify();
        })
        .detach();
        self.decorations.push(Decoration {
            id,
            owner,
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

impl Decoration {
    fn is_owned_by(&self, extension_id: &str) -> bool {
        self.owner
            .as_ref()
            .is_some_and(|owner| &*owner.extension_id == extension_id)
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
            let worktree_path = file.path().as_unix_str();
            let full_path = file.full_path(cx);
            let abs_path = file.as_local().map(|file| file.abs_path(cx));
            let buffer_id = buffer.remote_id();
            for decoration in &decorations.decorations {
                let matches = match &decoration.target.path {
                    DecorationPath::Host(path) => {
                        path_matches(path, &full_path, abs_path.as_deref())
                    }
                    DecorationPath::Worktree(path) => worktree_path_matches(path, worktree_path),
                };
                if !matches {
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

/// An extension's worktree-relative path names exactly the file at that path in its worktree. A
/// buffer outside every worktree (a single-file worktree has an empty relative path) never
/// matches.
pub(crate) fn worktree_path_matches(wanted: &str, worktree_path: &str) -> bool {
    !worktree_path.is_empty() && wanted == worktree_path
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

/// An extension's backend for one decoration: the first mount answers with the placed tree
/// without calling the extension; later mounts (a resync after a misfit) are `mount(id)` calls.
struct PlacedBackend {
    inner: Rc<dyn UiBackend>,
    placed: RefCell<Option<Tree>>,
}

impl UiBackend for PlacedBackend {
    fn extension_id(&self) -> &str {
        self.inner.extension_id()
    }

    fn mount(&self, surface_id: &str) -> LocalBoxFuture<'static, Result<Reply<Tree>>> {
        match self.placed.borrow_mut().take() {
            Some(value) => async move {
                Ok(Reply {
                    pushed: Vec::new(),
                    value,
                })
            }
            .boxed_local(),
            None => self.inner.mount(surface_id),
        }
    }

    fn handle_event(
        &self,
        surface_id: &str,
        event: UiEvent,
    ) -> LocalBoxFuture<'static, Result<Reply<Update>>> {
        self.inner.handle_event(surface_id, event)
    }

    fn list_rows(
        &self,
        surface_id: &str,
        list_node: NodeId,
        start: u32,
        count: u32,
    ) -> LocalBoxFuture<'static, Result<Reply<Vec<(u32, Tree)>>>> {
        self.inner.list_rows(surface_id, list_node, start, count)
    }

    fn diagnostics(&self) -> Option<ui_ext_wasm::diagnostics::Diagnostics> {
        self.inner.diagnostics()
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
            path: DecorationPath::Host(self.path),
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
