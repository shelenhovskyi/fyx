//! The texture fallback for canvases (Q14, plan 4.14). Painting a long draw list op by op costs
//! more than a frame has (`canvas::bench`: 16384 four-segment paths take ~200 ms), so only small
//! lists without paths are painted directly; every other canvas is rasterised once per update and
//! drawn as one image.
//!
//! - **Route:** [`route`]. Costs from the bench: a quad 1, a line 2, a text 8; direct while the sum
//!   is at most [`DIRECT_PAINT_BUDGET`] (about 2 ms), and never with a path.
//! - **Raster:** on the CPU, off the main thread, at the device-pixel size of the painted box, op
//!   by op (colours resolved for the theme on the main thread; GPUI's inside borders mimicked).
//!   Small rounded boxes, circles and lines get their per-pixel coverage, as GPUI's quad shader
//!   computes it; plain rectangles, large shapes and paths go through tiny-skia. Each distinct
//!   label is shaped and outlined once by usvg (Fyx's bundled fonts only), drawn once per colour
//!   and quarter-pixel x (the baseline on a whole pixel, as GPUI places glyphs), and blitted. A
//!   large texture is drawn as bands of rows on up to 8 threads, each band drawing the ops that
//!   reach it, in order. The bench's 16384-op lists take 6–40 ms at 2× (`canvas::bench`).
//! - **Updates:** the old texture keeps being drawn (scaled into the new box) until the new one is
//!   ready; at most one raster runs per canvas, and when it ends only the newest list is
//!   rasterised (the updates in between are dropped).
//!   A result whose generation is no longer the slot's is dropped.
//! - **Budget:** at most [`TEXTURE_BUDGET`] across all canvases, least recently painted first,
//!   never one painted in the current pass (on screen). One texture is at most
//!   [`MAX_TEXTURE_BYTES`]: a larger box is rasterised at a lower scale and drawn upscaled. A
//!   texture that does not fit beside the ones on screen is not kept: that canvas paints directly
//!   until room is freed. An evicted canvas rasterises again on its next paint.

use std::{collections::HashMap, fmt::Write as _, sync::Arc};

use anyhow::{Context as _, Result};
use gpui::{
    AnyWindowHandle, App, AppContext as _, Bounds, ContentMask, Corners, Global, Hsla, Pixels,
    RenderImage, Window,
};
use resvg::tiny_skia as skia;
use theme::{ActiveTheme as _, Appearance, Theme};
use ui_ext::{
    ColorToken,
    canvas::{CanvasColor, CanvasProps, CanvasTextSize, DrawOp, Paint, PathSegment, TextAnchor},
    v0_2::Props,
};

/// Bytes of canvas textures across every extension.
pub(crate) const TEXTURE_BUDGET: u64 = 128 << 20;
/// Bytes of one texture (BGRA): a 4096² canvas on a 2× screen is rasterised at 1×.
pub(crate) const MAX_TEXTURE_BYTES: u64 = 64 << 20;
/// Direct-paint cost units (a quad is 1) a frame may spend on one canvas.
pub(crate) const DIRECT_PAINT_BUDGET: u32 = 4000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Route {
    Direct,
    Texture,
}

/// How a draw list is painted. Scans at most [`DIRECT_PAINT_BUDGET`] + 1 ops.
pub(crate) fn route(props: &CanvasProps) -> Route {
    let mut cost = 0;
    for op in &props.ops {
        cost += match op {
            DrawOp::Path { .. } => return Route::Texture,
            DrawOp::Rect { .. } | DrawOp::RoundedRect { .. } | DrawOp::Circle { .. } => 1,
            DrawOp::Line { .. } => 2,
            DrawOp::Text { .. } => 8,
        };
        if cost > DIRECT_PAINT_BUDGET {
            return Route::Texture;
        }
    }
    Route::Direct
}

/// A canvas node: its surface view (entity id) and node id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct SlotKey {
    pub view: u64,
    pub node: u32,
}

/// Where a canvas's texture is kept.
#[derive(Clone)]
pub(crate) struct CanvasSlot {
    pub key: SlotKey,
    pub extension_id: Arc<str>,
}

/// What a texture was drawn from. Holding the `Arc`s means a pointer is never reused while it is
/// compared.
#[derive(Clone)]
struct TextureKey {
    props: Arc<Props>,
    device: (u32, u32),
    theme: Arc<Theme>,
    font_size: f32,
}

impl TextureKey {
    fn same(&self, other: &TextureKey) -> bool {
        Arc::ptr_eq(&self.props, &other.props)
            && self.device == other.device
            && Arc::ptr_eq(&self.theme, &other.theme)
            && self.font_size == other.font_size
    }
}

struct Texture {
    key: TextureKey,
    image: Arc<RenderImage>,
    bytes: u64,
    /// The windows whose sprite atlas holds it.
    windows: Vec<AnyWindowHandle>,
}

struct Slot {
    extension_id: Arc<str>,
    texture: Option<Texture>,
    /// The raster in flight: its generation and what it draws.
    running: Option<(u64, TextureKey)>,
    /// This list is painted directly: its texture did not fit, or failed.
    direct: Option<TextureKey>,
    over_budget: bool,
    last_used: u64,
    last_pass: u64,
}

/// Every canvas texture (global).
pub(crate) struct CanvasTextures {
    slots: HashMap<SlotKey, Slot>,
    used_bytes: u64,
    budget: u64,
    clock: u64,
    /// The pass paints stamp; a change (a finished raster) makes the next paint begin a new one,
    /// so the slots stamped with the current pass are the ones on screen.
    pass: u64,
    changed_since_pass: bool,
    next_generation: u64,
    fonts: Option<Arc<usvg::fontdb::Database>>,
    /// Tests: every raster waits for this before it starts.
    #[cfg(test)]
    hold: Option<futures::future::Shared<futures::channel::oneshot::Receiver<()>>>,
    #[cfg(test)]
    rasters_started: usize,
}

impl Default for CanvasTextures {
    fn default() -> Self {
        Self {
            slots: HashMap::default(),
            used_bytes: 0,
            budget: TEXTURE_BUDGET,
            clock: 0,
            pass: 0,
            changed_since_pass: false,
            next_generation: 0,
            fonts: None,
            #[cfg(test)]
            hold: None,
            #[cfg(test)]
            rasters_started: 0,
        }
    }
}

impl Global for CanvasTextures {}

/// What one paint of a texture-route canvas does.
struct Frame {
    image: Option<Arc<RenderImage>>,
    direct: bool,
    start: Option<u64>,
}

/// GPU atlas entries to release, with the windows that hold them.
type Drops = Vec<(Arc<RenderImage>, Vec<AnyWindowHandle>)>;

impl CanvasTextures {
    fn frame(
        &mut self,
        key: SlotKey,
        extension_id: &Arc<str>,
        wanted: &TextureKey,
        window: AnyWindowHandle,
    ) -> Frame {
        if self.changed_since_pass {
            self.pass += 1;
            self.changed_since_pass = false;
        }
        self.clock += 1;
        let slot = self.slots.entry(key).or_insert_with(|| Slot {
            extension_id: extension_id.clone(),
            texture: None,
            running: None,
            direct: None,
            over_budget: false,
            last_used: 0,
            last_pass: 0,
        });
        slot.last_used = self.clock;
        slot.last_pass = self.pass;
        if let Some(texture) = &mut slot.texture
            && texture.key.same(wanted)
        {
            if !texture.windows.contains(&window) {
                texture.windows.push(window);
            }
            return Frame {
                image: Some(texture.image.clone()),
                direct: false,
                start: None,
            };
        }
        if let Some(direct) = &slot.direct {
            if direct.same(wanted) {
                return Frame {
                    image: None,
                    direct: true,
                    start: None,
                };
            }
            slot.direct = None;
            slot.over_budget = false;
        }
        let start = slot.running.is_none().then(|| {
            self.next_generation += 1;
            slot.running = Some((self.next_generation, wanted.clone()));
            self.next_generation
        });
        let image = slot.texture.as_mut().map(|texture| {
            if !texture.windows.contains(&window) {
                texture.windows.push(window);
            }
            texture.image.clone()
        });
        Frame {
            image,
            direct: false,
            start,
        }
    }

    /// A raster ended: keep it if it is still the slot's and fits, replacing the old texture.
    fn finish(
        &mut self,
        key: SlotKey,
        generation: u64,
        result: Result<(RenderImage, u64)>,
    ) -> Drops {
        let mut drops = Drops::new();
        let Some(slot) = self.slots.get_mut(&key) else {
            return drops;
        };
        let Some((running, wanted)) = slot.running.take_if(|(running, _)| *running == generation)
        else {
            return drops;
        };
        debug_assert_eq!(running, generation);
        self.changed_since_pass = true;
        let (image, bytes) = match result {
            Ok(image) => image,
            Err(error) => {
                log::error!("fyx:ui: canvas raster of {}: {error:#}", slot.extension_id);
                slot.direct = Some(wanted);
                return drops;
            }
        };
        if let Some(old) = slot.texture.take() {
            self.used_bytes -= old.bytes;
            drops.push((old.image, old.windows));
        }
        if !self.evict_for(key, bytes, &mut drops) {
            let slot = self.slots.get_mut(&key).expect("still there");
            slot.direct = Some(wanted);
            slot.over_budget = true;
            log::warn!(
                "fyx:ui: canvas textures are over {} MiB on screen; {} paints a canvas directly",
                self.budget >> 20,
                slot.extension_id
            );
            return drops;
        }
        self.used_bytes += bytes;
        let slot = self.slots.get_mut(&key).expect("still there");
        slot.texture = Some(Texture {
            key: wanted,
            image: Arc::new(image),
            bytes,
            windows: Vec::new(),
        });
        drops
    }

    /// Makes room for `bytes` beside `keep`, least recently painted first, never a texture painted
    /// in the current pass. Evicts nothing unless that makes enough room. Drops slots that hold
    /// nothing and are off screen.
    fn evict_for(&mut self, keep: SlotKey, bytes: u64, drops: &mut Drops) -> bool {
        let pass = self.pass;
        self.slots.retain(|key, slot| {
            *key == keep
                || slot.texture.is_some()
                || slot.running.is_some()
                || slot.last_pass == pass
        });
        let mut evictable: Vec<(u64, SlotKey, u64)> = self
            .slots
            .iter()
            .filter(|(key, slot)| **key != keep && slot.last_pass != pass)
            .filter_map(|(key, slot)| Some((slot.last_used, *key, slot.texture.as_ref()?.bytes)))
            .collect();
        let freeable: u64 = evictable.iter().map(|(_, _, bytes)| bytes).sum();
        if self.used_bytes.saturating_sub(freeable) + bytes > self.budget {
            return false;
        }
        evictable.sort_by_key(|(last_used, _, _)| *last_used);
        for (_, key, _) in evictable {
            if self.used_bytes + bytes <= self.budget {
                break;
            }
            self.forget_texture(key, drops);
        }
        true
    }

    fn forget_texture(&mut self, key: SlotKey, drops: &mut Drops) {
        let Some(texture) = self
            .slots
            .get_mut(&key)
            .and_then(|slot| slot.texture.take())
        else {
            return;
        };
        self.used_bytes -= texture.bytes;
        drops.push((texture.image, texture.windows));
        // Room for what did not fit before.
        for slot in self.slots.values_mut() {
            if slot.over_budget {
                slot.direct = None;
                slot.over_budget = false;
            }
        }
    }

    fn remove_slot(&mut self, key: SlotKey, drops: &mut Drops) {
        self.forget_texture(key, drops);
        self.slots.remove(&key);
    }

    fn clear(&mut self, extension_id: &str) -> Drops {
        let mut drops = Drops::new();
        let keys: Vec<SlotKey> = self
            .slots
            .iter()
            .filter(|(_, slot)| &*slot.extension_id == extension_id)
            .map(|(key, _)| *key)
            .collect();
        for key in keys {
            self.remove_slot(key, &mut drops);
        }
        drops
    }
}

/// Releases the GPU atlas entries of textures no longer kept (after the current update).
fn release(drops: Drops, cx: &mut App) {
    if drops.is_empty() {
        return;
    }
    cx.defer(move |cx| {
        for (image, windows) in drops {
            for window in windows {
                cx.update_window(window, |_, window, _| window.drop_image(image.clone()))
                    .ok();
            }
        }
    });
}

/// An extension was unloaded: its canvas textures go.
pub(crate) fn clear(extension_id: &str, cx: &mut App) {
    if let Some(textures) = cx.try_global::<CanvasTextures>()
        && !textures.slots.is_empty()
    {
        let drops = cx.global_mut::<CanvasTextures>().clear(extension_id);
        release(drops, cx);
    }
}

/// The bench: whether the bench's slot holds a texture of `props`.
#[cfg(test)]
pub(crate) fn drawn_from_for_bench(props: &Arc<Props>, cx: &App) -> bool {
    cx.try_global::<CanvasTextures>().is_some_and(|textures| {
        textures
            .slots
            .get(&SlotKey { view: 0, node: 0 })
            .and_then(|slot| slot.texture.as_ref())
            .is_some_and(|texture| Arc::ptr_eq(&texture.key.props, props))
    })
}

/// Tests and the bench: the bundled UI and buffer fonts, read from `assets/fonts`.
#[cfg(test)]
pub(crate) fn test_fonts() -> Arc<usvg::fontdb::Database> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts");
    let mut fonts = usvg::fontdb::Database::new();
    for (dir, file) in [
        ("ibm-plex-sans", "IBMPlexSans-Regular.ttf"),
        ("lilex", "Lilex-Regular.ttf"),
    ] {
        fonts.load_font_data(std::fs::read(root.join(dir).join(file)).expect("bundled font"));
    }
    fonts.set_sans_serif_family("IBM Plex Sans");
    fonts.set_monospace_family("Lilex");
    Arc::new(fonts)
}

/// The device-pixel size a box is rasterised at: the box × the scale factor, within
/// [`MAX_TEXTURE_BYTES`].
pub(crate) fn device_size(bounds: Bounds<Pixels>, scale_factor: f32) -> (u32, u32) {
    let width = (f32::from(bounds.size.width) * scale_factor)
        .round()
        .max(1.);
    let height = (f32::from(bounds.size.height) * scale_factor)
        .round()
        .max(1.);
    let bytes = width as f64 * height as f64 * 4.;
    let fit = (MAX_TEXTURE_BYTES as f64 / bytes).sqrt().min(1.);
    (
        ((width as f64 * fit).floor() as u32).max(1),
        ((height as f64 * fit).floor() as u32).max(1),
    )
}

/// Paints a canvas through its texture, or directly when [`route`] says so (or with no slot).
pub(crate) fn paint(
    slot: Option<&CanvasSlot>,
    shared: &Arc<Props>,
    props: &CanvasProps,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(slot) = slot else {
        return crate::canvas::paint_ops(props, bounds, window, cx);
    };
    if route(props) == Route::Direct {
        if cx
            .try_global::<CanvasTextures>()
            .is_some_and(|textures| textures.slots.contains_key(&slot.key))
        {
            let mut drops = Drops::new();
            cx.global_mut::<CanvasTextures>()
                .remove_slot(slot.key, &mut drops);
            release(drops, cx);
        }
        return crate::canvas::paint_ops(props, bounds, window, cx);
    }
    let wanted = TextureKey {
        props: shared.clone(),
        device: device_size(bounds, window.scale_factor()),
        theme: cx.theme().clone(),
        font_size: f32::from(theme::theme_settings(cx).ui_font_size(cx)),
    };
    let frame = cx.default_global::<CanvasTextures>().frame(
        slot.key,
        &slot.extension_id,
        &wanted,
        window.window_handle(),
    );
    if let Some(generation) = frame.start {
        start_raster(slot.key, generation, &wanted, props, cx);
    }
    if frame.direct {
        crate::canvas::paint_ops(props, bounds, window, cx);
    } else if let Some(image) = frame.image {
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            window
                .paint_image(bounds, bounds, Corners::default(), image, 0, false)
                .unwrap_or_else(|error| log::error!("fyx:ui: canvas texture: {error:#}"));
        });
    }
}

fn start_raster(
    key: SlotKey,
    generation: u64,
    wanted: &TextureKey,
    props: &CanvasProps,
    cx: &mut App,
) {
    let palette = Palette::resolve(props, wanted.font_size, cx);
    let fonts = match cx.global::<CanvasTextures>().fonts.clone() {
        Some(fonts) => fonts,
        None => {
            let fonts = crate::images::shared_fonts(cx);
            cx.global_mut::<CanvasTextures>().fonts = Some(fonts.clone());
            fonts
        }
    };
    #[cfg(test)]
    let hold = {
        let textures = cx.global_mut::<CanvasTextures>();
        textures.rasters_started += 1;
        textures.hold.clone()
    };
    let shared = wanted.props.clone();
    let device = wanted.device;
    let task = cx.background_spawn(async move {
        #[cfg(test)]
        if let Some(hold) = hold {
            hold.await.ok();
        }
        let ui_ext::v0_2::Props::Canvas(props) = &*shared else {
            anyhow::bail!("not a canvas");
        };
        rasterize(props, &palette, device, fonts)
    });
    cx.spawn(async move |cx| {
        let result = task.await;
        cx.update(|cx| {
            let drops = cx
                .global_mut::<CanvasTextures>()
                .finish(key, generation, result);
            release(drops, cx);
            cx.refresh_windows();
        });
    })
    .detach();
}

/// The theme's colours for the tokens a draw list uses, resolved on the main thread.
pub(crate) struct Palette {
    tokens: HashMap<ColorToken, Hsla>,
    appearance: Appearance,
    /// The UI font size in logical pixels: text sizes are this × 0.85, 1 or 1.2.
    font_size: f32,
}

impl Palette {
    pub(crate) fn resolve(props: &CanvasProps, font_size: f32, cx: &App) -> Self {
        let mut tokens = HashMap::default();
        let mut add = |paint: &Paint| {
            if let CanvasColor::Token(token) = paint.color {
                tokens
                    .entry(token)
                    .or_insert_with(|| crate::view::text_color(token, cx).color(cx));
            }
        };
        for op in &props.ops {
            match op {
                DrawOp::Line { stroke, .. } => add(&stroke.paint),
                DrawOp::Rect { fill, stroke, .. }
                | DrawOp::RoundedRect { fill, stroke, .. }
                | DrawOp::Circle { fill, stroke, .. }
                | DrawOp::Path { fill, stroke, .. } => {
                    fill.iter().for_each(&mut add);
                    stroke.iter().for_each(|stroke| add(&stroke.paint));
                }
                DrawOp::Text { paint, .. } => add(paint),
            }
        }
        Self {
            tokens,
            appearance: cx.theme().appearance(),
            font_size,
        }
    }

    fn color(&self, paint: Paint) -> Hsla {
        let color = match paint.color {
            CanvasColor::Token(token) => self
                .tokens
                .get(&token)
                .copied()
                .unwrap_or_else(gpui::transparent_black),
            CanvasColor::Data(series) => crate::canvas::data_color(series, self.appearance),
        };
        color.opacity(f32::from(paint.opacity.min(100)) / 100.)
    }
}

fn n(value: f32) -> f32 {
    if value.is_finite() { value } else { 0. }
}

/// A rectangle with non-negative sizes.
fn normalized(rect: &ui_ext::canvas::Rect) -> (f32, f32, f32, f32) {
    let (x, width) = if rect.width.0 < 0. {
        (rect.x.0 + rect.width.0, -rect.width.0)
    } else {
        (rect.x.0, rect.width.0)
    };
    let (y, height) = if rect.height.0 < 0. {
        (rect.y.0 + rect.height.0, -rect.height.0)
    } else {
        (rect.y.0, rect.height.0)
    };
    (n(x), n(y), n(width), n(height))
}

/// Text as XML character data: escaped, without the control characters XML forbids.
fn write_text(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\t' | '\n' | '\r' => out.push(' '),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
}

/// A label: what a text op shapes to. Ops that show the same label share one shaping.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct LabelKey<'a> {
    text: &'a str,
    size: CanvasTextSize,
    anchor: TextAnchor,
    monospace: bool,
}

/// Labels as an SVG for usvg to shape and outline, each `<text id="l<label>">` at the origin.
fn labels_svg<'a>(
    labels: impl IntoIterator<Item = (usize, LabelKey<'a>)>,
    font_size: f32,
) -> String {
    let mut out = String::from(r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1">"#);
    for (id, label) in labels {
        let factor = match label.size {
            CanvasTextSize::Small => 0.85,
            CanvasTextSize::Medium => 1.,
            CanvasTextSize::Large => 1.2,
        };
        let anchor = match label.anchor {
            TextAnchor::Start => "start",
            TextAnchor::Middle => "middle",
            TextAnchor::End => "end",
        };
        let family = if label.monospace {
            "Lilex"
        } else {
            "IBM Plex Sans"
        };
        write!(
            out,
            r#"<text id="l{id}" x="0" y="0" font-family="{family}" font-size="{}" text-anchor="{anchor}" xml:space="preserve">"#,
            font_size * factor,
        )
        .ok();
        write_text(&mut out, label.text);
        out.push_str("</text>");
    }
    out.push_str("</svg>");
    out
}

/// Shapes `svg` ([`labels_svg`]) and returns the tree with, for each label in it, the path of
/// child indices to its text node.
fn shape_labels(
    svg: &str,
    fonts: Arc<usvg::fontdb::Database>,
) -> Result<(usvg::Tree, Vec<(usize, Vec<usize>)>)> {
    let options = usvg::Options {
        resources_dir: None,
        fontdb: fonts,
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_str(svg, &options).context("shaping the canvas text")?;
    fn walk(group: &usvg::Group, trail: &mut Vec<usize>, found: &mut Vec<(usize, Vec<usize>)>) {
        for (child, node) in group.children().iter().enumerate() {
            trail.push(child);
            match node {
                usvg::Node::Text(text) => {
                    if let Some(id) = text.id().strip_prefix('l').and_then(|id| id.parse().ok()) {
                        found.push((id, trail.clone()));
                    }
                }
                usvg::Node::Group(group) => walk(group, trail, found),
                _ => {}
            }
            trail.pop();
        }
    }
    let mut found = Vec::new();
    walk(tree.root(), &mut Vec::new(), &mut found);
    Ok((tree, found))
}

fn text_node<'a>(tree: &'a usvg::Tree, trail: &[usize]) -> Option<&'a usvg::Text> {
    let mut group = tree.root();
    let (last, parents) = trail.split_last()?;
    for child in parents {
        match group.children().get(*child)? {
            usvg::Node::Group(inner) => group = inner,
            _ => return None,
        }
    }
    match group.children().get(*last)? {
        usvg::Node::Text(text) => Some(text),
        _ => None,
    }
}

fn skia_paint(color: Hsla) -> skia::Paint<'static> {
    let rgba = color.to_rgb();
    let channel = |value: f32| (value.clamp(0., 1.) * 255.).round() as u8;
    let mut paint = skia::Paint::default();
    paint.set_color_rgba8(
        channel(rgba.r),
        channel(rgba.g),
        channel(rgba.b),
        channel(rgba.a),
    );
    paint.anti_alias = true;
    paint
}

/// A rounded rectangle (`radius` 0: a plain one) as a subpath.
fn push_rounded(path: &mut skia::PathBuilder, x: f32, y: f32, width: f32, height: f32, r: f32) {
    if r <= 0. {
        if let Some(rect) = skia::Rect::from_xywh(x, y, width, height) {
            path.push_rect(rect);
        }
        return;
    }
    // Quarter circles as cubics.
    let k = r * 0.552_284_8;
    let (right, bottom) = (x + width, y + height);
    path.move_to(x + r, y);
    path.line_to(right - r, y);
    path.cubic_to(right - r + k, y, right, y + r - k, right, y + r);
    path.line_to(right, bottom - r);
    path.cubic_to(
        right,
        bottom - r + k,
        right - r + k,
        bottom,
        right - r,
        bottom,
    );
    path.line_to(x + r, bottom);
    path.cubic_to(x + r - k, bottom, x, bottom - r + k, x, bottom - r);
    path.line_to(x, y + r);
    path.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    path.close();
}

/// The canvas → device mapping of one band of rows, `oy` device pixels down.
#[derive(Clone, Copy)]
struct Mapping {
    sx: f32,
    sy: f32,
    /// Lengths (stroke widths, radii): the mean of the two scales.
    s: f32,
    oy: f32,
}

impl Mapping {
    fn x(&self, x: f32) -> f32 {
        x * self.sx
    }

    fn y(&self, y: f32) -> f32 {
        y * self.sy - self.oy
    }
}

/// An op's rows in device pixels (before the band offset), padded for antialiasing, or `None`
/// when it draws nothing.
fn op_rows(op: &DrawOp, at: Mapping, sprite: Option<Placement>) -> Option<(f32, f32)> {
    let (top, bottom) = match op {
        DrawOp::Line { from, to, stroke } => {
            let pad = n(stroke.width.0) * at.s / 2.;
            let (a, b) = (at.y(n(from.y.0)), at.y(n(to.y.0)));
            (a.min(b) - pad, a.max(b) + pad)
        }
        DrawOp::Rect { bounds, .. } | DrawOp::RoundedRect { bounds, .. } => {
            let (_, y, _, height) = normalized(bounds);
            (at.y(y), at.y(y + height))
        }
        DrawOp::Circle { center, radius, .. } => {
            let r = n(radius.0).max(0.);
            (at.y(n(center.y.0) - r), at.y(n(center.y.0) + r))
        }
        DrawOp::Path {
            segments, stroke, ..
        } => {
            let pad = stroke.map_or(0., |stroke| n(stroke.width.0) * at.s);
            let (mut top, mut bottom) = (at.y(0.), at.y(0.));
            let mut first = !matches!(segments.first(), Some(PathSegment::MoveTo(_)));
            let mut add = |p: &ui_ext::canvas::Point| {
                let y = at.y(n(p.y.0));
                if !first {
                    (top, bottom) = (y, y);
                    first = true;
                }
                top = top.min(y);
                bottom = bottom.max(y);
            };
            for segment in segments {
                match segment {
                    PathSegment::MoveTo(p) | PathSegment::LineTo(p) => add(p),
                    PathSegment::QuadTo(c, p) => {
                        add(c);
                        add(p)
                    }
                    PathSegment::CubicTo(a, b, p) => {
                        add(a);
                        add(b);
                        add(p)
                    }
                    PathSegment::Close => {}
                }
            }
            // A miter can reach a few widths past the outline.
            (top - pad * 4., bottom + pad * 4.)
        }
        DrawOp::Text { .. } => {
            let (sprite, _, y) = sprite?;
            let top = (y + sprite.top) as f32 - at.oy;
            (top, top + sprite.pixmap.height() as f32)
        }
    };
    Some((top - 1., bottom + 1.))
}

/// tiny-skia logs a warning for every path with no area it is asked to fill.
fn fillable(path: &skia::Path) -> bool {
    let bounds = path.bounds();
    bounds.width() > f32::EPSILON && bounds.height() > f32::EPSILON
}

/// Device pixels up to which a rounded box, circle or line is drawn by its per-pixel coverage
/// (like GPUI's quad shader) rather than through tiny-skia, whose fixed cost per path dominates
/// small shapes.
const MAX_COVERAGE_AREA: f32 = 16384.;

/// How much of a pixel centred `offset` from a span's middle the span of half-length `half`
/// covers, along one axis.
fn overlap(offset: f32, half: f32) -> f32 {
    ((offset + 0.5).min(half) - (offset - 0.5).max(-half)).clamp(0., 1.)
}

/// The coverage of the pixel centred at `(px, py)` by a box centred at `(cx, cy)` with half
/// extents `(hx, hy)` and corner radius `r`: from its signed distance, as GPUI's quads do, and
/// exact along straight edges.
fn box_coverage(px: f32, py: f32, cx: f32, cy: f32, hx: f32, hy: f32, r: f32) -> f32 {
    if hx <= 0. || hy <= 0. {
        return 0.;
    }
    let r = r.min(hx).min(hy);
    if r <= 0. {
        return overlap(px - cx, hx) * overlap(py - cy, hy);
    }
    let qx = (px - cx).abs() - hx + r;
    let qy = (py - cy).abs() - hy + r;
    let outside = (qx.max(0.) * qx.max(0.) + qy.max(0.) * qy.max(0.)).sqrt();
    let distance = outside + qx.max(qy).min(0.) - r;
    (0.5 - distance).clamp(0., 1.)
}

/// Source-over of a premultiplied colour at `coverage` onto a premultiplied RGBA8 pixel.
/// `value / 255`, rounded, for `value` up to 255².
fn div255(value: u32) -> u32 {
    let value = value + 128;
    (value + (value >> 8)) >> 8
}

/// Source-over of a premultiplied RGBA8 colour at `coverage` (0 to 256) onto a premultiplied
/// RGBA8 pixel.
fn blend(pixel: &mut [u8], color: [u32; 4], coverage: u32) {
    if coverage >= 256 && color[3] == 255 {
        for (channel, source) in pixel.iter_mut().zip(color) {
            *channel = source as u8;
        }
        return;
    }
    let source = color.map(|channel| (channel * coverage) >> 8);
    let keep = 255 - source[3];
    for (channel, source) in pixel.iter_mut().zip(source) {
        *channel = (source + div255(*channel as u32 * keep)).min(255) as u8;
    }
}

fn premultiplied(color: Hsla) -> [u32; 4] {
    let rgba = color.to_rgb();
    let alpha = rgba.a.clamp(0., 1.);
    let channel = |value: f32| (value.clamp(0., 1.) * alpha * 255.).round() as u32;
    [
        channel(rgba.r),
        channel(rgba.g),
        channel(rgba.b),
        (alpha * 255.).round() as u32,
    ]
}

/// A label drawn once for one colour and quarter-pixel position, blitted wherever it is shown.
struct Sprite {
    pixmap: skia::Pixmap,
    /// Its top-left corner from the label's origin (whole device pixels).
    left: i64,
    top: i64,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct SpriteKey {
    label: usize,
    color: [u8; 4],
    /// The origin's x within its pixel, in quarters (GPUI's four subpixel variants).
    quarter: u8,
}

/// Where a text op shows its sprite: the sprite and the label origin in whole device pixels
/// (before the band offset).
type Placement<'a> = (&'a Sprite, i64, i64);

/// A text op's sprite key and origin: x to a quarter pixel, the baseline to a whole one (as GPUI
/// places glyphs).
fn text_origin(
    op: &DrawOp,
    at: Mapping,
    label: usize,
    color: Hsla,
) -> Option<(SpriteKey, i64, i64)> {
    let DrawOp::Text { at: origin, .. } = op else {
        return None;
    };
    let quarters = (at.x(n(origin.x.0)) * 4.).floor();
    let y = (n(origin.y.0) * at.sy).round();
    if !quarters.is_finite() || !y.is_finite() {
        return None;
    }
    let quarters = quarters as i64;
    let rgba = color.to_rgb();
    let channel = |value: f32| (value.clamp(0., 1.) * 255.).round() as u8;
    Some((
        SpriteKey {
            label,
            color: [
                channel(rgba.r),
                channel(rgba.g),
                channel(rgba.b),
                channel(rgba.a),
            ],
            quarter: quarters.rem_euclid(4) as u8,
        },
        quarters.div_euclid(4),
        y as i64,
    ))
}

fn draw_sprite(text: &usvg::Text, key: SpriteKey, at: Mapping) -> Option<Sprite> {
    let bounds = text.abs_bounding_box();
    let shift = f32::from(key.quarter) / 4.;
    let left = (bounds.left() * at.sx + shift).floor() as i64 - 1;
    let top = (bounds.top() * at.sy).floor() as i64 - 1;
    let right = (bounds.right() * at.sx + shift).ceil() as i64 + 1;
    let bottom = (bounds.bottom() * at.sy).ceil() as i64 + 1;
    let mut pixmap = skia::Pixmap::new(
        u32::try_from(right - left).ok()?,
        u32::try_from(bottom - top).ok()?,
    )?;
    let [r, g, b, a] = key.color;
    let mut paint = skia::Paint::default();
    paint.set_color_rgba8(r, g, b, a);
    paint.anti_alias = true;
    let base = skia::Transform::from_row(at.sx, 0., 0., at.sy, shift - left as f32, -top as f32);
    fn walk(
        group: &usvg::Group,
        pixmap: &mut skia::Pixmap,
        paint: &skia::Paint,
        base: skia::Transform,
    ) {
        for node in group.children() {
            match node {
                usvg::Node::Path(path) => pixmap.fill_path(
                    path.data(),
                    paint,
                    skia::FillRule::Winding,
                    base.pre_concat(path.abs_transform()),
                    None,
                ),
                usvg::Node::Group(group) => walk(group, pixmap, paint, base),
                _ => {}
            }
        }
    }
    walk(text.flattened(), &mut pixmap, &paint, base);
    Some(Sprite { pixmap, left, top })
}

struct Raster<'a> {
    pixmap: skia::PixmapMut<'a>,
    at: Mapping,
}

impl Raster<'_> {
    /// Blends `color` at `coverage(pixel centre)` over the pixels of `bounds` (device, band
    /// offset applied), padded by one pixel for antialiasing.
    /// Blends `under` then `over` at the coverages `coverage(pixel centre)` returns over the
    /// pixels of `bounds` (device, band offset applied), padded by one pixel for antialiasing.
    fn cover(
        &mut self,
        (x0, y0, x1, y1): (f32, f32, f32, f32),
        under: Option<Hsla>,
        over: Option<Hsla>,
        coverage: impl Fn(f32, f32) -> (f32, f32),
    ) {
        let (width, height) = (self.pixmap.width() as i64, self.pixmap.height() as i64);
        let xs = ((x0 - 1.).floor() as i64).max(0);
        let xe = ((x1 + 1.).ceil() as i64).min(width);
        let ys = ((y0 - 1.).floor() as i64).max(0);
        let ye = ((y1 + 1.).ceil() as i64).min(height);
        if xs >= xe || ys >= ye {
            return;
        }
        let (under, over) = (under.map(premultiplied), over.map(premultiplied));
        let fixed = |k: f32| (k.clamp(0., 1.) * 256.).round() as u32;
        let data = self.pixmap.data_mut();
        for y in ys..ye {
            let py = y as f32 + 0.5;
            let row = (y * width) as usize * 4;
            for x in xs..xe {
                let (k_under, k_over) = coverage(x as f32 + 0.5, py);
                let pixel = &mut data[row + x as usize * 4..][..4];
                if let Some(color) = under
                    && k_under > 0.
                {
                    blend(pixel, color, fixed(k_under));
                }
                if let Some(color) = over
                    && k_over > 0.
                {
                    blend(pixel, color, fixed(k_over));
                }
            }
        }
    }

    /// A sprite with its label origin at device pixel `(x, y)` (before the band offset).
    fn blit(&mut self, sprite: &Sprite, x: i64, y: i64) {
        let (width, height) = (self.pixmap.width() as i64, self.pixmap.height() as i64);
        let left = x + sprite.left;
        let top = y + sprite.top - self.at.oy as i64;
        let (sw, sh) = (sprite.pixmap.width() as i64, sprite.pixmap.height() as i64);
        let source = sprite.pixmap.data();
        let data = self.pixmap.data_mut();
        for sy in (0.max(-top))..sh.min(height - top) {
            let dy = top + sy;
            for sx in (0.max(-left))..sw.min(width - left) {
                let from = ((sy * sw + sx) * 4) as usize;
                let alpha = source[from + 3];
                if alpha == 0 {
                    continue;
                }
                let to = ((dy * width + left + sx) * 4) as usize;
                let keep = 255 - alpha as u32;
                for channel in 0..4 {
                    let blended = source[from + channel] as u32
                        + (data[to + channel] as u32 * keep + 127) / 255;
                    data[to + channel] = blended.min(255) as u8;
                }
            }
        }
    }

    fn fill(&mut self, path: Option<skia::Path>, color: Hsla, rule: skia::FillRule) {
        if let Some(path) = path
            && fillable(&path)
        {
            self.pixmap.fill_path(
                &path,
                &skia_paint(color),
                rule,
                skia::Transform::identity(),
                None,
            );
        }
    }

    /// A box like GPUI's quad: the fill over the whole box, the border inside it.
    fn draw_box(
        &mut self,
        (x, y, width, height): (f32, f32, f32, f32),
        radius: f32,
        fill: Option<Hsla>,
        stroke: Option<(f32, Hsla)>,
    ) {
        let at = self.at;
        let (x, y, width, height) = (at.x(x), at.y(y), width * at.sx, height * at.sy);
        let radius = radius * at.s;
        if width <= 0. || height <= 0. {
            return;
        }
        // Rounded boxes and circles: per-pixel coverage, fill and border in one pass. Plain
        // rectangles stay with tiny-skia, whose rect fill is faster still.
        if radius > 0. && width * height <= MAX_COVERAGE_AREA {
            let (cx, cy, hx, hy) = (x + width / 2., y + height / 2., width / 2., height / 2.);
            let bounds = (x, y, x + width, y + height);
            let border = stroke.map(|(stroke_width, _)| stroke_width * at.s);
            let inner = border
                .filter(|border| border * 2. < width.min(height))
                .map(|border| (hx - border, hy - border, (radius - border).max(0.)));
            self.cover(bounds, fill, stroke.map(|(_, color)| color), |px, py| {
                let outer = box_coverage(px, py, cx, cy, hx, hy, radius);
                let ring = match inner {
                    Some((ix, iy, ir)) => outer - box_coverage(px, py, cx, cy, ix, iy, ir),
                    None => outer,
                };
                (outer, ring)
            });
            return;
        }
        if let Some(fill) = fill {
            if radius <= 0. {
                if let Some(rect) = skia::Rect::from_xywh(x, y, width, height) {
                    self.pixmap.fill_rect(
                        rect,
                        &skia_paint(fill),
                        skia::Transform::identity(),
                        None,
                    );
                }
            } else {
                let mut path = skia::PathBuilder::new();
                push_rounded(&mut path, x, y, width, height, radius);
                self.fill(path.finish(), fill, skia::FillRule::Winding);
            }
        }
        if let Some((stroke_width, color)) = stroke {
            let border = stroke_width * at.s;
            if radius <= 0. && border * 2. < width.min(height) {
                // Four strips: much cheaper than a path.
                let paint = skia_paint(color);
                let inner = height - 2. * border;
                for (x, y, width, height) in [
                    (x, y, width, border),
                    (x, y + height - border, width, border),
                    (x, y + border, border, inner),
                    (x + width - border, y + border, border, inner),
                ] {
                    if let Some(rect) = skia::Rect::from_xywh(x, y, width, height) {
                        self.pixmap
                            .fill_rect(rect, &paint, skia::Transform::identity(), None);
                    }
                }
                return;
            }
            let mut path = skia::PathBuilder::new();
            push_rounded(&mut path, x, y, width, height, radius);
            if border * 2. < width.min(height) {
                // The ring between the box and the box inset by the border.
                push_rounded(
                    &mut path,
                    x + border,
                    y + border,
                    width - 2. * border,
                    height - 2. * border,
                    (radius - border).max(0.),
                );
            }
            self.fill(path.finish(), color, skia::FillRule::EvenOdd);
        }
    }

    fn draw_line(&mut self, from: (f32, f32), to: (f32, f32), width: f32, color: Hsla) {
        let at = self.at;
        let (x0, y0) = (at.x(from.0), at.y(from.1));
        let (x1, y1) = (at.x(to.0), at.y(to.1));
        let (dx, dy) = (x1 - x0, y1 - y0);
        let length = (dx * dx + dy * dy).sqrt();
        if length == 0. || !length.is_finite() {
            return;
        }
        // A butt-capped stroke is a quad around the segment.
        let half = width * at.s / 2.;
        if (dx.abs() + 2. * half) * (dy.abs() + 2. * half) <= MAX_COVERAGE_AREA {
            let (ux, uy) = (dx / length, dy / length);
            let (mx, my) = ((x0 + x1) / 2., (y0 + y1) / 2.);
            let half_length = length / 2.;
            let bounds = (
                x0.min(x1) - half,
                y0.min(y1) - half,
                x0.max(x1) + half,
                y0.max(y1) + half,
            );
            self.cover(bounds, None, Some(color), |px, py| {
                let (rx, ry) = (px - mx, py - my);
                // Along the segment and across it, each as the pixel's overlap with the stroke.
                let along = rx * ux + ry * uy;
                let across = ry * ux - rx * uy;
                (0., overlap(along, half_length) * overlap(across, half))
            });
            return;
        }
        let (nx, ny) = (-dy / length * half, dx / length * half);
        let mut path = skia::PathBuilder::new();
        path.move_to(x0 + nx, y0 + ny);
        path.line_to(x1 + nx, y1 + ny);
        path.line_to(x1 - nx, y1 - ny);
        path.line_to(x0 - nx, y0 - ny);
        path.close();
        self.fill(path.finish(), color, skia::FillRule::Winding);
    }

    fn draw_path(
        &mut self,
        segments: &[PathSegment],
        fill: Option<Hsla>,
        stroke: Option<(f32, Hsla)>,
    ) {
        let at = self.at;
        let point = |p: &ui_ext::canvas::Point| (at.x(n(p.x.0)), at.y(n(p.y.0)));
        let mut path = skia::PathBuilder::new();
        if !matches!(segments.first(), Some(PathSegment::MoveTo(_))) {
            path.move_to(at.x(0.), at.y(0.));
        }
        for segment in segments {
            match segment {
                PathSegment::MoveTo(p) => {
                    let (x, y) = point(p);
                    path.move_to(x, y)
                }
                PathSegment::LineTo(p) => {
                    let (x, y) = point(p);
                    path.line_to(x, y)
                }
                PathSegment::QuadTo(c, p) => {
                    let ((cx, cy), (x, y)) = (point(c), point(p));
                    path.quad_to(cx, cy, x, y)
                }
                PathSegment::CubicTo(a, b, p) => {
                    let ((ax, ay), (bx, by), (x, y)) = (point(a), point(b), point(p));
                    path.cubic_to(ax, ay, bx, by, x, y)
                }
                PathSegment::Close => path.close(),
            }
        }
        let Some(path) = path.finish() else {
            return;
        };
        if let Some(fill) = fill
            && fillable(&path)
        {
            self.pixmap.fill_path(
                &path,
                &skia_paint(fill),
                skia::FillRule::Winding,
                skia::Transform::identity(),
                None,
            );
        }
        if let Some((width, color)) = stroke {
            let stroke = skia::Stroke {
                width: width * at.s,
                ..skia::Stroke::default()
            };
            self.pixmap.stroke_path(
                &path,
                &skia_paint(color),
                &stroke,
                skia::Transform::identity(),
                None,
            );
        }
    }

    fn draw_op(&mut self, op: &DrawOp, palette: &Palette, sprite: Option<Placement>) {
        let stroke_of = |stroke: &Option<ui_ext::canvas::Stroke>| {
            stroke.map(|stroke| (n(stroke.width.0), palette.color(stroke.paint)))
        };
        match op {
            DrawOp::Line { from, to, stroke } => self.draw_line(
                (n(from.x.0), n(from.y.0)),
                (n(to.x.0), n(to.y.0)),
                n(stroke.width.0),
                palette.color(stroke.paint),
            ),
            DrawOp::Rect {
                bounds,
                fill,
                stroke,
            } => self.draw_box(
                normalized(bounds),
                0.,
                fill.map(|fill| palette.color(fill)),
                stroke_of(stroke),
            ),
            DrawOp::RoundedRect {
                bounds,
                radius,
                fill,
                stroke,
            } => {
                let rect = normalized(bounds);
                let radius = n(radius.0).min(rect.2 / 2.).min(rect.3 / 2.).max(0.);
                self.draw_box(
                    rect,
                    radius,
                    fill.map(|fill| palette.color(fill)),
                    stroke_of(stroke),
                )
            }
            DrawOp::Circle {
                center,
                radius,
                fill,
                stroke,
            } => {
                let r = n(radius.0).max(0.);
                let (cx, cy) = (n(center.x.0), n(center.y.0));
                self.draw_box(
                    (cx - r, cy - r, 2. * r, 2. * r),
                    r,
                    fill.map(|fill| palette.color(fill)),
                    stroke_of(stroke),
                )
            }
            DrawOp::Path {
                segments,
                fill,
                stroke,
            } => self.draw_path(
                segments,
                fill.map(|fill| palette.color(fill)),
                stroke_of(stroke),
            ),
            DrawOp::Text { .. } => {
                if let Some((sprite, x, y)) = sprite {
                    self.blit(sprite, x, y);
                }
            }
        }
    }
}

/// Threads one raster uses: its rows are split into bands drawn at once, and its labels are
/// shaped in as many parts.
fn raster_threads() -> usize {
    std::thread::available_parallelism().map_or(1, |threads| threads.get().clamp(1, 8))
}

/// Draws a draw list at `device` pixels, as BGRA, op by op in order: small boxes, circles and
/// lines by their per-pixel coverage, other shapes with tiny-skia. Every label is shaped and
/// outlined once by usvg with `fonts` only (Fyx's bundled fonts), drawn once per colour and
/// quarter-pixel position, and blitted. Large textures are drawn as bands of rows on several
/// threads, each band drawing only the ops that reach it.
pub(crate) fn rasterize(
    props: &CanvasProps,
    palette: &Palette,
    device: (u32, u32),
    fonts: Arc<usvg::fontdb::Database>,
) -> Result<(RenderImage, u64)> {
    rasterize_on(props, palette, device, fonts, raster_threads())
}

/// [`rasterize`] on at most `threads` threads.
pub(crate) fn rasterize_on(
    props: &CanvasProps,
    palette: &Palette,
    (width, height): (u32, u32),
    fonts: Arc<usvg::fontdb::Database>,
    threads: usize,
) -> Result<(RenderImage, u64)> {
    let threads = threads.max(1);
    let sx = width as f32 / props.width as f32;
    let sy = height as f32 / props.height as f32;
    let at = Mapping {
        sx,
        sy,
        s: (sx + sy) / 2.,
        oy: 0.,
    };

    // The labels, each shaped once, in parts on several threads.
    let mut label_ids: HashMap<LabelKey, usize> = HashMap::default();
    let mut op_labels: Vec<Option<usize>> = Vec::with_capacity(props.ops.len());
    for op in &props.ops {
        op_labels.push(match op {
            DrawOp::Text {
                text,
                size,
                anchor,
                monospace,
                ..
            } if !text.trim().is_empty() => {
                let key = LabelKey {
                    text,
                    size: *size,
                    anchor: *anchor,
                    monospace: *monospace,
                };
                let next = label_ids.len();
                Some(*label_ids.entry(key).or_insert(next))
            }
            _ => None,
        });
    }
    let mut labels: Vec<(usize, LabelKey)> = label_ids.into_iter().map(|(k, id)| (id, k)).collect();
    labels.sort_by_key(|(id, _)| *id);
    let parts = if labels.is_empty() {
        Vec::new()
    } else {
        let chunk = labels
            .len()
            .div_ceil(threads.min(labels.len().div_ceil(64)).max(1));
        let svgs: Vec<String> = labels
            .chunks(chunk)
            .map(|part| labels_svg(part.iter().copied(), palette.font_size))
            .collect();
        std::thread::scope(|scope| {
            let shaping: Vec<_> = svgs
                .iter()
                .map(|svg| {
                    let fonts = fonts.clone();
                    scope.spawn(move || shape_labels(svg, fonts))
                })
                .collect();
            shaping
                .into_iter()
                .map(|part| {
                    part.join()
                        .unwrap_or_else(|_| Err(anyhow::anyhow!("panicked")))
                })
                .collect::<Result<Vec<_>>>()
        })?
    };
    let mut shaped: Vec<Option<&usvg::Text>> = vec![None; labels.len()];
    for (tree, found) in &parts {
        for (id, trail) in found {
            if let Some(slot) = shaped.get_mut(*id) {
                *slot = text_node(tree, trail);
            }
        }
    }

    // Each label drawn once per colour and quarter-pixel position, in parts on several threads.
    let mut sprite_ids: HashMap<SpriteKey, usize> = HashMap::default();
    let mut op_sprites: Vec<Option<(usize, i64, i64)>> = Vec::with_capacity(props.ops.len());
    for (index, op) in props.ops.iter().enumerate() {
        let placed = match (op, op_labels[index]) {
            (DrawOp::Text { paint, .. }, Some(label)) if shaped[label].is_some() => {
                text_origin(op, at, label, palette.color(*paint))
            }
            _ => None,
        };
        op_sprites.push(placed.map(|(key, x, y)| {
            let next = sprite_ids.len();
            (*sprite_ids.entry(key).or_insert(next), x, y)
        }));
    }
    let mut keys: Vec<(usize, SpriteKey)> = sprite_ids.into_iter().map(|(k, id)| (id, k)).collect();
    keys.sort_by_key(|(id, _)| *id);
    let draw_sprites = |part: &[(usize, SpriteKey)]| -> Vec<Option<Sprite>> {
        part.iter()
            .map(|(_, key)| draw_sprite(shaped[key.label]?, *key, at))
            .collect()
    };
    let sprites: Vec<Option<Sprite>> = if keys.len() <= 64 || threads == 1 {
        draw_sprites(&keys)
    } else {
        let chunk = keys.len().div_ceil(threads);
        std::thread::scope(|scope| {
            let drawing: Vec<_> = keys
                .chunks(chunk)
                .map(|part| scope.spawn(move || draw_sprites(part)))
                .collect();
            drawing
                .into_iter()
                .flat_map(|part| part.join().unwrap_or_default())
                .collect()
        })
    };
    let sprite_of = |index: usize| -> Option<Placement> {
        let (id, x, y) = op_sprites[index]?;
        Some((sprites.get(id)?.as_ref()?, x, y))
    };

    // Each op's rows, once.
    let rows: Vec<Option<(f32, f32)>> = props
        .ops
        .iter()
        .enumerate()
        .map(|(index, op)| op_rows(op, at, sprite_of(index)))
        .collect();

    let bands = threads.min((height / 128).max(1) as usize).max(1);
    let band_height = height.div_ceil(bands as u32);
    let mut data = vec![0u8; width as usize * height as usize * 4];
    // Each band draws into its own rows of the texture, then turns them into straight BGRA.
    let draw_band = |top: u32, rows_here: &mut [u8]| -> Result<()> {
        let band_rows = (rows_here.len() / 4 / width as usize) as u32;
        let mut raster = Raster {
            pixmap: skia::PixmapMut::from_bytes(rows_here, width, band_rows)
                .context("the canvas texture's rows")?,
            at: Mapping {
                oy: top as f32,
                ..at
            },
        };
        let (band_top, band_bottom) = (top as f32, (top + band_rows) as f32);
        for (index, op) in props.ops.iter().enumerate() {
            if let Some((op_top, op_bottom)) = rows[index]
                && op_bottom >= band_top
                && op_top <= band_bottom
            {
                raster.draw_op(op, palette, sprite_of(index));
            }
        }
        for pixel in rows_here.chunks_exact_mut(4) {
            match pixel[3] {
                0 => {}
                255 => pixel.swap(0, 2),
                _ => gpui::swap_rgba_pa_to_bgra(pixel),
            }
        }
        Ok(())
    };
    let band_bytes = band_height as usize * width as usize * 4;
    if bands == 1 {
        draw_band(0, &mut data)?;
    } else {
        std::thread::scope(|scope| {
            let drawing: Vec<_> = data
                .chunks_mut(band_bytes)
                .enumerate()
                .map(|(band, rows_here)| {
                    let top = band as u32 * band_height;
                    scope.spawn(move || draw_band(top, rows_here))
                })
                .collect();
            drawing.into_iter().try_for_each(|band| {
                band.join()
                    .unwrap_or_else(|_| Err(anyhow::anyhow!("a raster band panicked")))
            })
        })?;
    }
    let bytes = data.len() as u64;
    let buffer = image::RgbaImage::from_raw(width, height, data).context("the canvas texture")?;
    Ok((RenderImage::new(vec![image::Frame::new(buffer)]), bytes))
}

#[cfg(test)]
mod tests {
    use futures::FutureExt as _;
    use gpui::{
        AvailableSpace, ElementId, IntoElement as _, ParentElement as _, Styled as _,
        TestAppContext, VisualTestContext, div, point, px, size,
    };
    use ui_ext::canvas::{CanvasFit, DataColor, Num, Point as CanvasPoint, Rect, Stroke};

    use super::*;

    fn paint(series: DataColor) -> Paint {
        Paint {
            color: CanvasColor::Data(series),
            opacity: 100,
        }
    }

    fn rect(x: f32) -> DrawOp {
        DrawOp::Rect {
            bounds: Rect {
                x: Num(x),
                y: Num(0.),
                width: Num(10.),
                height: Num(10.),
            },
            fill: Some(paint(DataColor::Series2)),
            stroke: Some(Stroke {
                paint: paint(DataColor::Series1),
                width: Num(2.),
            }),
        }
    }

    fn p(x: f32, y: f32) -> CanvasPoint {
        CanvasPoint {
            x: Num(x),
            y: Num(y),
        }
    }

    fn path() -> DrawOp {
        DrawOp::Path {
            segments: vec![
                PathSegment::MoveTo(p(0., 0.)),
                PathSegment::LineTo(p(50., 0.)),
                PathSegment::LineTo(p(0., 50.)),
                PathSegment::Close,
            ],
            fill: Some(paint(DataColor::Series3)),
            stroke: None,
        }
    }

    fn text(value: &str) -> DrawOp {
        DrawOp::Text {
            at: p(5., 20.),
            text: value.into(),
            paint: paint(DataColor::Series1),
            size: CanvasTextSize::Medium,
            anchor: TextAnchor::Middle,
            monospace: false,
        }
    }

    fn canvas_props(ops: Vec<DrawOp>) -> CanvasProps {
        CanvasProps {
            width: 100,
            height: 100,
            description: "chart".into(),
            ops,
            clickable: false,
            fit: CanvasFit::Clip,
        }
    }

    fn shared(ops: Vec<DrawOp>) -> Arc<Props> {
        Arc::new(Props::Canvas(canvas_props(ops)))
    }

    fn palette() -> Palette {
        Palette {
            tokens: HashMap::default(),
            appearance: Appearance::Light,
            font_size: 14.,
        }
    }

    /// The cost model: paths always go through a texture; quads, lines and texts up to the budget
    /// are painted directly.
    #[test]
    fn test_route_follows_the_cost_model() {
        let route_of = |ops: Vec<DrawOp>| route(&canvas_props(ops));
        assert_eq!(route_of(vec![rect(0.); 10]), Route::Direct);
        assert_eq!(route_of(vec![path()]), Route::Texture, "a path");
        assert_eq!(route_of(vec![rect(0.); 4000]), Route::Direct);
        assert_eq!(route_of(vec![rect(0.); 4001]), Route::Texture);
        assert_eq!(route_of(vec![text("1"); 500]), Route::Direct);
        assert_eq!(route_of(vec![text("1"); 501]), Route::Texture);
        let line = DrawOp::Line {
            from: p(0., 0.),
            to: p(9., 9.),
            stroke: Stroke {
                paint: paint(DataColor::Series1),
                width: Num(1.),
            },
        };
        assert_eq!(route_of(vec![line.clone(); 2000]), Route::Direct);
        assert_eq!(route_of(vec![line; 2001]), Route::Texture);
    }

    #[test]
    fn test_one_texture_is_capped() {
        let bounds = |side: f32| Bounds::new(point(px(0.), px(0.)), size(px(side), px(side / 2.)));
        assert_eq!(device_size(bounds(100.), 2.), (200, 100));
        // 4096 × 2048 at 2× would be 128 MiB: drawn at about 1.4×, within 64 MiB.
        let (width, height) = device_size(bounds(4096.), 2.);
        assert!(width as u64 * height as u64 * 4 <= MAX_TEXTURE_BYTES);
        assert!(width >= 5780 && height >= 2890, "{width} x {height}");
    }

    /// GPUI's quads put the border inside the box: so does the texture.
    #[test]
    fn test_texture_draws_boxes_like_gpui() {
        let props = canvas_props(vec![rect(0.)]);
        let (image, bytes) = rasterize(
            &props,
            &palette(),
            (200, 200),
            Arc::new(usvg::fontdb::Database::new()),
        )
        .unwrap();
        assert_eq!(bytes, 200 * 200 * 4);
        let data = image.as_bytes(0).unwrap();
        let pixel = |x: usize, y: usize| -> [u8; 4] {
            let at = (y * 200 + x) * 4;
            data[at..at + 4].try_into().unwrap()
        };
        let bgra = |series| {
            let rgba = crate::canvas::data_color(series, Appearance::Light).to_rgb();
            let c = |v: f32| (v * 255.).round() as i32;
            [c(rgba.b), c(rgba.g), c(rgba.r), 255]
        };
        let close = |got: [u8; 4], want: [i32; 4]| {
            got.iter()
                .zip(want)
                .all(|(got, want)| (*got as i32 - want).abs() <= 2)
        };
        // 2× scale: the 2-unit border is 4 px, inside the 20 px box.
        let border = bgra(DataColor::Series1);
        assert!(close(pixel(1, 1), border), "{:?}", pixel(1, 1));
        assert!(close(pixel(18, 10), border), "{:?}", pixel(18, 10));
        assert!(
            close(pixel(10, 10), bgra(DataColor::Series2)),
            "{:?}",
            pixel(10, 10)
        );
        assert_eq!(pixel(21, 10), [0, 0, 0, 0], "nothing outside the box");
    }

    #[test]
    fn test_text_is_escaped_into_the_svg() {
        let props = canvas_props(vec![text("<a> & \"b\"\u{0}\u{7}\n")]);
        let DrawOp::Text {
            text,
            size,
            anchor,
            monospace,
            ..
        } = &props.ops[0]
        else {
            unreachable!()
        };
        let label = LabelKey {
            text,
            size: *size,
            anchor: *anchor,
            monospace: *monospace,
        };
        let svg = labels_svg([(0, label)], 14.);
        assert!(svg.contains(">&lt;a&gt; &amp; \"b\" </text>"), "{svg}");
        rasterize(
            &props,
            &palette(),
            (100, 100),
            Arc::new(usvg::fontdb::Database::new()),
        )
        .unwrap();
    }

    /// Text in the bundled font, at its anchor, in op order: a later box covers it.
    #[test]
    fn test_text_draws_with_the_bundled_fonts_in_op_order() {
        let ink = |ops: Vec<DrawOp>| {
            let (image, _) =
                rasterize(&canvas_props(ops), &palette(), (100, 100), test_fonts()).unwrap();
            let data = image.as_bytes(0).unwrap().to_vec();
            let inked = |x0: usize, x1: usize| {
                (0..100)
                    .flat_map(|y| (x0..x1).map(move |x| (y * 100 + x) * 4 + 3))
                    .filter(|at| data[*at] > 0)
                    .count()
            };
            (inked(0, 50), inked(50, 100))
        };
        let label = |anchor: TextAnchor| DrawOp::Text {
            at: p(50., 30.),
            text: "Wide".into(),
            paint: paint(DataColor::Series1),
            size: CanvasTextSize::Large,
            anchor,
            monospace: false,
        };
        let (left, right) = ink(vec![label(TextAnchor::Start)]);
        assert!(
            left == 0 && right > 20,
            "start: right of x ({left}, {right})"
        );
        let (left, right) = ink(vec![label(TextAnchor::End)]);
        assert!(left > 20 && right == 0, "end: left of x ({left}, {right})");
        let (left, right) = ink(vec![label(TextAnchor::Middle)]);
        assert!(
            left > 10 && right > 10,
            "middle: both sides ({left}, {right})"
        );
        let cover = DrawOp::Rect {
            bounds: Rect {
                x: Num(0.),
                y: Num(0.),
                width: Num(100.),
                height: Num(100.),
            },
            fill: Some(paint(DataColor::Series2)),
            stroke: None,
        };
        let (image, _) = rasterize(
            &canvas_props(vec![label(TextAnchor::Start), cover]),
            &palette(),
            (100, 100),
            test_fonts(),
        )
        .unwrap();
        let data = image.as_bytes(0).unwrap();
        let first = &data[0..4];
        assert!(
            data.chunks_exact(4).all(|pixel| pixel == first),
            "the later box covers the text"
        );
    }

    /// Bands drawn on several threads give the same pixels as one band: shapes and labels that
    /// cross a band edge, in op order.
    #[test]
    fn test_bands_match_one_band() {
        let mut ops = Vec::new();
        for i in 0..64 {
            let y = i as f32 * 15.;
            ops.push(DrawOp::Circle {
                center: p(20. + i as f32, y),
                radius: Num(9.),
                fill: Some(paint(DataColor::Series4)),
                stroke: Some(Stroke {
                    paint: paint(DataColor::Series5),
                    width: Num(1.5),
                }),
            });
            ops.push(rect(i as f32 * 3.));
            ops.push(DrawOp::Line {
                from: p(0., y),
                to: p(100., 1000. - y),
                stroke: Stroke {
                    paint: paint(DataColor::Series6),
                    width: Num(1.),
                },
            });
            ops.push(DrawOp::Text {
                at: p(50., y + 4.),
                text: (i % 7).to_string(),
                paint: paint(DataColor::Series1),
                size: CanvasTextSize::Large,
                anchor: TextAnchor::Middle,
                monospace: i % 2 == 0,
            });
        }
        ops.push(DrawOp::Path {
            segments: vec![
                PathSegment::MoveTo(p(0., 0.)),
                PathSegment::CubicTo(p(100., 200.), p(0., 800.), p(100., 1000.)),
            ],
            fill: None,
            stroke: Some(Stroke {
                paint: paint(DataColor::Series2),
                width: Num(3.),
            }),
        });
        let props = CanvasProps {
            width: 100,
            height: 1000,
            ..canvas_props(ops)
        };
        let draw = |threads| {
            let (image, _) =
                rasterize_on(&props, &palette(), (200, 2000), test_fonts(), threads).unwrap();
            image.as_bytes(0).unwrap().to_vec()
        };
        let one = draw(1);
        assert!(one.chunks_exact(4).any(|pixel| pixel[3] > 0));
        // tiny-skia clips each path to its band, so antialiasing differs a little along shapes that
        // cross a band edge (long thin lines most): a few per cent of the pixels, very few by much.
        let eight = draw(8);
        let (mut differ, mut far) = (0, 0);
        for (a, b) in one.chunks_exact(4).zip(eight.chunks_exact(4)) {
            let most = a
                .iter()
                .zip(b)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap_or(0);
            differ += usize::from(most > 0);
            far += usize::from(most > 32);
        }
        let inked = one.chunks_exact(4).filter(|pixel| pixel[3] > 0).count();
        assert!(differ * 20 <= inked, "{differ} of {inked} pixels differ");
        assert!(
            far * 500 <= inked,
            "{far} of {inked} pixels differ by more than 32"
        );
    }

    /// The rows an op is culled to cover every row it inks.
    #[test]
    fn test_op_rows_cover_the_ink() {
        let ops = [
            rect(10.),
            DrawOp::Circle {
                center: p(50., 50.),
                radius: Num(20.),
                fill: None,
                stroke: Some(Stroke {
                    paint: paint(DataColor::Series5),
                    width: Num(2.),
                }),
            },
            DrawOp::Line {
                from: p(10., 90.),
                to: p(90., 10.),
                stroke: Stroke {
                    paint: paint(DataColor::Series6),
                    width: Num(8.),
                },
            },
            // A sharp miter.
            DrawOp::Path {
                segments: vec![
                    PathSegment::MoveTo(p(20., 80.)),
                    PathSegment::LineTo(p(50., 30.)),
                    PathSegment::LineTo(p(80., 80.)),
                ],
                fill: None,
                stroke: Some(Stroke {
                    paint: paint(DataColor::Series2),
                    width: Num(10.),
                }),
            },
            DrawOp::Path {
                segments: vec![
                    PathSegment::MoveTo(p(10., 50.)),
                    PathSegment::QuadTo(p(50., -20.), p(90., 50.)),
                    PathSegment::Close,
                ],
                fill: Some(paint(DataColor::Series3)),
                stroke: None,
            },
            DrawOp::Text {
                at: p(50., 60.),
                text: "Ågjy".into(),
                paint: paint(DataColor::Series1),
                size: CanvasTextSize::Large,
                anchor: TextAnchor::Middle,
                monospace: false,
            },
        ];
        for op in ops {
            let props = canvas_props(vec![op.clone()]);
            let (image, _) = rasterize_on(&props, &palette(), (200, 200), test_fonts(), 1).unwrap();
            let data = image.as_bytes(0).unwrap();
            let inked: Vec<usize> = (0..200)
                .filter(|y| (0..200).any(|x| data[(y * 200 + x) * 4 + 3] > 0))
                .collect();
            let (first, last) = (inked[0] as f32, (*inked.last().unwrap() + 1) as f32);
            let at = Mapping {
                sx: 2.,
                sy: 2.,
                s: 2.,
                oy: 0.,
            };
            let label = match &op {
                DrawOp::Text {
                    text,
                    size,
                    anchor,
                    monospace,
                    ..
                } => {
                    let key = LabelKey {
                        text,
                        size: *size,
                        anchor: *anchor,
                        monospace: *monospace,
                    };
                    Some(shape_labels(&labels_svg([(0, key)], 14.), test_fonts()).unwrap())
                }
                _ => None,
            };
            let sprite = label.as_ref().and_then(|(tree, found)| {
                let text = text_node(tree, &found[0].1)?;
                let (key, x, y) = text_origin(&op, at, 0, gpui::black())?;
                Some((draw_sprite(text, key, at)?, x, y))
            });
            let (top, bottom) =
                op_rows(&op, at, sprite.as_ref().map(|(s, x, y)| (s, *x, *y))).unwrap();
            assert!(
                top <= first && bottom >= last,
                "{op:?}: culled to {top}..{bottom}, inks {first}..{last}"
            );
        }
    }

    fn slot(node: u32) -> SlotKey {
        SlotKey { view: 1, node }
    }

    /// The canvases inside a view (tooltips need one).
    struct Canvases(Vec<(u32, Arc<Props>)>);

    impl gpui::Render for Canvases {
        fn render(
            &mut self,
            _: &mut Window,
            _: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            div()
                .flex()
                .flex_col()
                .children(self.0.iter().map(|(node, props)| {
                    crate::canvas::canvas_element(
                        ElementId::Integer(*node as u64),
                        props.clone(),
                        Some(CanvasSlot {
                            key: slot(*node),
                            extension_id: "fyx-test".into(),
                        }),
                        None,
                    )
                }))
        }
    }

    fn draw(vcx: &mut VisualTestContext, canvases: &[(u32, &Arc<Props>)]) {
        let canvases: Vec<(u32, Arc<Props>)> = canvases
            .iter()
            .map(|(node, props)| (*node, Arc::clone(props)))
            .collect();
        vcx.draw(
            point(px(0.), px(0.)),
            size(
                AvailableSpace::Definite(px(400.)),
                AvailableSpace::Definite(px(800.)),
            ),
            move |_, cx| cx.new(|_| Canvases(canvases)).into_any_element(),
        );
    }

    fn textures<R>(vcx: &mut VisualTestContext, f: impl FnOnce(&CanvasTextures) -> R) -> R {
        vcx.update(|_, cx| f(cx.default_global::<CanvasTextures>()))
    }

    fn drawn_from(vcx: &mut VisualTestContext, node: u32) -> Option<Arc<Props>> {
        textures(vcx, |textures| {
            let slot = textures.slots.get(&slot(node))?;
            Some(slot.texture.as_ref()?.key.props.clone())
        })
    }

    fn running(vcx: &mut VisualTestContext, node: u32) -> Option<Arc<Props>> {
        textures(vcx, |textures| {
            let slot = textures.slots.get(&slot(node))?;
            Some(slot.running.as_ref()?.1.props.clone())
        })
    }

    fn started(vcx: &mut VisualTestContext) -> usize {
        textures(vcx, |textures| textures.rasters_started)
    }

    fn same(a: &Option<Arc<Props>>, b: &Arc<Props>) -> bool {
        a.as_ref().is_some_and(|a| Arc::ptr_eq(a, b))
    }

    /// An update keeps the old texture on screen until the new one is ready; one raster runs at
    /// a time, and the newest list is drawn after it.
    #[gpui::test]
    fn test_an_update_keeps_the_old_texture_until_the_new_one_is_ready(cx: &mut TestAppContext) {
        crate::tests::init_test(cx);
        let vcx = cx.add_empty_window();
        let first = shared(vec![path()]);
        draw(vcx, &[(1, &first)]);
        assert!(
            drawn_from(vcx, 1).is_none(),
            "rasterised off the main thread"
        );
        vcx.run_until_parked();
        assert!(same(&drawn_from(vcx, 1), &first));

        let (release, hold) = futures::channel::oneshot::channel();
        vcx.update(|_, cx| cx.global_mut::<CanvasTextures>().hold = Some(hold.shared()));
        let second = shared(vec![path(), rect(60.)]);
        draw(vcx, &[(1, &second)]);
        vcx.run_until_parked();
        assert!(same(&drawn_from(vcx, 1), &first), "the old texture stays");
        assert!(same(&running(vcx, 1), &second));

        let third = shared(vec![path(), rect(70.)]);
        let before = started(vcx);
        draw(vcx, &[(1, &third)]);
        draw(vcx, &[(1, &third)]);
        assert_eq!(started(vcx), before, "one raster at a time");

        release.send(()).unwrap();
        vcx.run_until_parked();
        assert!(same(&drawn_from(vcx, 1), &second));
        draw(vcx, &[(1, &third)]);
        vcx.run_until_parked();
        assert!(same(&drawn_from(vcx, 1), &third), "then the newest list");
        // The same list at the same size is not rasterised again.
        let before = started(vcx);
        draw(vcx, &[(1, &third)]);
        assert_eq!(started(vcx), before);

        // A small list without paths is painted directly, and its slot goes.
        let small = shared(vec![rect(0.)]);
        draw(vcx, &[(1, &small)]);
        assert!(textures(vcx, |textures| !textures
            .slots
            .contains_key(&slot(1))));
        assert_eq!(textures(vcx, |textures| textures.used_bytes), 0);
    }

    /// The budget: least recently painted first, never one on screen; an evicted canvas
    /// rasterises again when it is painted; one that does not fit beside those on screen paints
    /// directly; an unloaded extension's textures go.
    #[gpui::test]
    fn test_textures_are_evicted_at_the_budget(cx: &mut TestAppContext) {
        crate::tests::init_test(cx);
        let vcx = cx.add_empty_window();
        let canvases: Vec<Arc<Props>> = (0..4)
            .map(|i| shared(vec![path(), rect(i as f32)]))
            .collect();
        draw(vcx, &[(0, &canvases[0])]);
        vcx.run_until_parked();
        let one = textures(vcx, |textures| textures.used_bytes);
        assert!(one > 0);
        vcx.update(|_, cx| cx.global_mut::<CanvasTextures>().budget = one * 5 / 2);

        draw(vcx, &[(1, &canvases[1])]);
        vcx.run_until_parked();
        draw(vcx, &[(2, &canvases[2])]);
        vcx.run_until_parked();
        assert!(
            drawn_from(vcx, 0).is_none(),
            "the least recently painted goes"
        );
        assert!(drawn_from(vcx, 1).is_some() && drawn_from(vcx, 2).is_some());
        assert_eq!(textures(vcx, |textures| textures.used_bytes), one * 2);

        // Painted again: rasterised again, and now 1 is the oldest.
        draw(vcx, &[(0, &canvases[0])]);
        vcx.run_until_parked();
        assert!(same(&drawn_from(vcx, 0), &canvases[0]));
        assert!(drawn_from(vcx, 1).is_none());

        // Three on screen at once, room for two: the third paints directly.
        let all = [(0, &canvases[0]), (2, &canvases[2]), (3, &canvases[3])];
        draw(vcx, &all);
        vcx.run_until_parked();
        assert!(drawn_from(vcx, 0).is_some() && drawn_from(vcx, 2).is_some());
        assert!(drawn_from(vcx, 3).is_none());
        let direct = textures(vcx, |textures| {
            let slot = &textures.slots[&slot(3)];
            slot.direct.is_some() && slot.running.is_none()
        });
        assert!(direct, "over budget: painted directly");
        let before = started(vcx);
        draw(vcx, &all);
        vcx.run_until_parked();
        assert_eq!(started(vcx), before, "and not rasterised in a loop");

        vcx.update(|_, cx| clear("fyx-test", cx));
        textures(vcx, |textures| {
            assert!(textures.slots.is_empty());
            assert_eq!(textures.used_bytes, 0);
        });
    }
}
