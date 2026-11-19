//! The texture fallback for canvases (Q14, plan 4.14). Painting a long draw list op by op costs
//! more than a frame has (`canvas::bench`: 16384 four-segment paths take ~200 ms), so only small
//! lists without paths are painted directly; every other canvas is rasterised once per update and
//! drawn as one image.
//!
//! - **Route:** [`route`]. Costs from the bench: a quad 1, a line 2, a text 8; direct while the sum
//!   is at most [`DIRECT_PAINT_BUDGET`] (about 2 ms), and never with a path.
//! - **Raster:** on the CPU, off the main thread. The draw list becomes an SVG (colours resolved
//!   for the theme on the main thread; GPUI's inside borders mimicked), which resvg draws at the
//!   device-pixel size of the painted box, with Fyx's bundled fonts only.
//! - **Updates:** the old texture keeps being drawn (scaled into the new box) until the new one is
//!   ready; at most one raster runs per canvas, and the newest list is rasterised when it ends.
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
        let svg = to_svg(props, &palette);
        rasterize(&svg, device, fonts)
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

/// ` fill="#rrggbb" fill-opacity="a"` (or `stroke`).
fn write_paint(out: &mut String, attribute: &str, color: Hsla) {
    let rgba = color.to_rgb();
    let channel = |value: f32| (value.clamp(0., 1.) * 255.).round() as u8;
    write!(
        out,
        r##" {attribute}="#{:02x}{:02x}{:02x}" {attribute}-opacity="{}""##,
        channel(rgba.r),
        channel(rgba.g),
        channel(rgba.b),
        rgba.a.clamp(0., 1.)
    )
    .ok();
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

/// A box like GPUI's quad: the fill over the whole box, the border inside it.
fn write_box(
    out: &mut String,
    (x, y, width, height): (f32, f32, f32, f32),
    radius: f32,
    fill: Option<Hsla>,
    stroke: Option<(f32, Hsla)>,
) {
    if let Some(fill) = fill {
        write!(
            out,
            r#"<rect x="{x}" y="{y}" width="{width}" height="{height}" rx="{radius}""#
        )
        .ok();
        write_paint(out, "fill", fill);
        out.push_str("/>");
    }
    if let Some((stroke_width, color)) = stroke {
        if stroke_width * 2. >= width.min(height) {
            // The border covers the whole box.
            write!(
                out,
                r#"<rect x="{x}" y="{y}" width="{width}" height="{height}" rx="{radius}""#
            )
            .ok();
            write_paint(out, "fill", color);
        } else {
            let half = stroke_width / 2.;
            write!(
                out,
                r#"<rect x="{}" y="{}" width="{}" height="{}" rx="{}" fill="none" stroke-width="{stroke_width}""#,
                x + half,
                y + half,
                width - stroke_width,
                height - stroke_width,
                (radius - half).max(0.),
            )
            .ok();
            write_paint(out, "stroke", color);
        }
        out.push_str("/>");
    }
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

/// The draw list as an SVG in canvas coordinates.
pub(crate) fn to_svg(props: &CanvasProps, palette: &Palette) -> String {
    let mut out = String::with_capacity(256 + props.ops.len() * 128);
    write!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{0}" height="{1}" viewBox="0 0 {0} {1}">"#,
        props.width, props.height
    )
    .ok();
    for op in &props.ops {
        match op {
            DrawOp::Line { from, to, stroke } => {
                write!(
                    out,
                    r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke-width="{}""#,
                    n(from.x.0),
                    n(from.y.0),
                    n(to.x.0),
                    n(to.y.0),
                    n(stroke.width.0)
                )
                .ok();
                write_paint(&mut out, "stroke", palette.color(stroke.paint));
                out.push_str("/>");
            }
            DrawOp::Rect {
                bounds,
                fill,
                stroke,
            } => write_box(
                &mut out,
                normalized(bounds),
                0.,
                fill.map(|fill| palette.color(fill)),
                stroke.map(|stroke| (n(stroke.width.0), palette.color(stroke.paint))),
            ),
            DrawOp::RoundedRect {
                bounds,
                radius,
                fill,
                stroke,
            } => {
                let rect = normalized(bounds);
                let radius = n(radius.0).min(rect.2 / 2.).min(rect.3 / 2.).max(0.);
                write_box(
                    &mut out,
                    rect,
                    radius,
                    fill.map(|fill| palette.color(fill)),
                    stroke.map(|stroke| (n(stroke.width.0), palette.color(stroke.paint))),
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
                write_box(
                    &mut out,
                    (cx - r, cy - r, 2. * r, 2. * r),
                    r,
                    fill.map(|fill| palette.color(fill)),
                    stroke.map(|stroke| (n(stroke.width.0), palette.color(stroke.paint))),
                )
            }
            DrawOp::Path {
                segments,
                fill,
                stroke,
            } => {
                out.push_str(r#"<path d=""#);
                if !matches!(segments.first(), Some(PathSegment::MoveTo(_))) {
                    out.push_str("M0 0");
                }
                for segment in segments {
                    match segment {
                        PathSegment::MoveTo(p) => write!(out, "M{} {}", n(p.x.0), n(p.y.0)),
                        PathSegment::LineTo(p) => write!(out, "L{} {}", n(p.x.0), n(p.y.0)),
                        PathSegment::QuadTo(c, p) => {
                            write!(out, "Q{} {} {} {}", n(c.x.0), n(c.y.0), n(p.x.0), n(p.y.0))
                        }
                        PathSegment::CubicTo(a, b, p) => write!(
                            out,
                            "C{} {} {} {} {} {}",
                            n(a.x.0),
                            n(a.y.0),
                            n(b.x.0),
                            n(b.y.0),
                            n(p.x.0),
                            n(p.y.0)
                        ),
                        PathSegment::Close => write!(out, "Z"),
                    }
                    .ok();
                }
                out.push('"');
                match fill {
                    Some(fill) => write_paint(&mut out, "fill", palette.color(*fill)),
                    None => out.push_str(r#" fill="none""#),
                }
                if let Some(stroke) = stroke {
                    write!(out, r#" stroke-width="{}""#, n(stroke.width.0)).ok();
                    write_paint(&mut out, "stroke", palette.color(stroke.paint));
                }
                out.push_str("/>");
            }
            DrawOp::Text {
                at,
                text,
                paint,
                size,
                anchor,
                monospace,
            } => {
                let factor = match size {
                    CanvasTextSize::Small => 0.85,
                    CanvasTextSize::Medium => 1.,
                    CanvasTextSize::Large => 1.2,
                };
                let anchor = match anchor {
                    TextAnchor::Start => "start",
                    TextAnchor::Middle => "middle",
                    TextAnchor::End => "end",
                };
                let family = if *monospace { "Lilex" } else { "IBM Plex Sans" };
                write!(
                    out,
                    r#"<text x="{}" y="{}" font-family="{family}" font-size="{}" text-anchor="{anchor}" xml:space="preserve""#,
                    n(at.x.0),
                    n(at.y.0),
                    palette.font_size * factor,
                )
                .ok();
                write_paint(&mut out, "fill", palette.color(*paint));
                out.push('>');
                write_text(&mut out, text);
                out.push_str("</text>");
            }
        }
    }
    out.push_str("</svg>");
    out
}

/// Draws a canvas SVG at `device` pixels, as BGRA. No href is resolved; text uses `fonts` only.
pub(crate) fn rasterize(
    svg: &str,
    (width, height): (u32, u32),
    fonts: Arc<usvg::fontdb::Database>,
) -> Result<(RenderImage, u64)> {
    let options = usvg::Options {
        resources_dir: None,
        fontdb: fonts,
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_str(svg, &options).context("parsing the canvas SVG")?;
    let size = tree.size();
    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(width, height).context("allocating the canvas texture")?;
    let transform = resvg::tiny_skia::Transform::from_scale(
        width as f32 / size.width(),
        height as f32 / size.height(),
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let mut data = pixmap.take();
    for pixel in data.chunks_exact_mut(4) {
        gpui::swap_rgba_pa_to_bgra(pixel);
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
        let svg = to_svg(&props, &palette());
        let (image, bytes) =
            rasterize(&svg, (200, 200), Arc::new(usvg::fontdb::Database::new())).unwrap();
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
        let svg = to_svg(&props, &palette());
        assert!(svg.contains(">&lt;a&gt; &amp; \"b\" </text>"), "{svg}");
        rasterize(&svg, (100, 100), Arc::new(usvg::fontdb::Database::new())).unwrap();
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
