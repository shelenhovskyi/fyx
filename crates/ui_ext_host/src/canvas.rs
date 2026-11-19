//! Canvas nodes (fyx:ui 0.3, design.md §13, plan 4.14): the host paints the retained draw list,
//! with no WASM call inside `render()`.
//!
//! - One GPUI `canvas` element per node, painted inside a content mask of its box, so nothing is
//!   drawn outside it. Rectangles, rounded rectangles and circles are quads; lines and paths are
//!   `PathBuilder` paths (paths fill non-zero); text is a shaped line in the UI font (or the buffer
//!   font) at the host's small, medium or large size.
//! - **Texture (Q14):** only small lists without paths are painted op by op each frame; the others
//!   are rasterised once per update, off the main thread, and drawn as one image
//!   (`canvas_texture`).
//! - **Fit:** `clip` keeps the box at its size, and a narrower parent clips it; `scale-down` shrinks
//!   the box to the parent's width, keeping its aspect ratio, and never grows it.
//! - **Colours:** theme tokens, or the eight `data-color` series. The series follow the
//!   Okabe–Ito palette (distinct under common colour-vision deficiencies), with a lighter variant
//!   on a dark theme. A theme change repaints with no call to the extension.
//! - **Clicks** on a `clickable` canvas map the pointer back through the painted box (which already
//!   includes the panel's scroll) and the `scale-down` factor, into canvas coordinates.

use std::{cell::Cell, rc::Rc, sync::Arc};

use gpui::{
    App, BorderStyle, Bounds, ContentMask, Corners, Edges, Hsla, PathBuilder, Pixels, Point,
    SharedString, TextAlign, TextRun, Window, canvas as gpui_canvas, point, prelude::*, px, quad,
    transparent_black,
};
use theme::{ActiveTheme as _, Appearance};
use ui_ext::canvas::{
    CanvasColor, CanvasFit, CanvasProps, CanvasTextSize, DataColor, DrawOp, Paint, PathSegment,
    TextAnchor,
};

/// The colour of a paint on the active theme, with its opacity.
pub(crate) fn paint_color(paint: Paint, cx: &App) -> Hsla {
    let color = match paint.color {
        CanvasColor::Token(token) => crate::view::text_color(token, cx).color(cx),
        CanvasColor::Data(series) => data_color(series, cx.theme().appearance()),
    };
    color.opacity(f32::from(paint.opacity.min(100)) / 100.)
}

/// The eight series: Okabe–Ito, lightened on a dark theme.
pub(crate) fn data_color(series: DataColor, appearance: Appearance) -> Hsla {
    let rgb = match series {
        DataColor::Series1 => 0x0072B2,
        DataColor::Series2 => 0xE69F00,
        DataColor::Series3 => 0x009E73,
        DataColor::Series4 => 0xCC79A7,
        DataColor::Series5 => 0x56B4E9,
        DataColor::Series6 => 0xD55E00,
        DataColor::Series7 => 0xF0E442,
        DataColor::Series8 => 0x999999,
    };
    let color: Hsla = gpui::rgb(rgb).into();
    match appearance {
        Appearance::Light => color,
        Appearance::Dark => Hsla {
            l: (color.l + (1. - color.l) * 0.25).min(0.9),
            ..color
        },
    }
}

/// The scale `scale-down` draws at in a box `available` wide; 1 for `clip`.
pub(crate) fn fit_scale(props: &CanvasProps, painted_width: Pixels) -> f32 {
    match props.fit {
        CanvasFit::Clip => 1.,
        CanvasFit::ScaleDown => (f32::from(painted_width) / props.width as f32).min(1.),
    }
}

/// A pointer position in the window, as canvas coordinates of a canvas painted at `bounds`.
pub(crate) fn to_canvas(
    props: &CanvasProps,
    bounds: Bounds<Pixels>,
    position: Point<Pixels>,
) -> Option<(f32, f32)> {
    if !bounds.contains(&position) {
        return None;
    }
    let scale = fit_scale(props, bounds.size.width);
    let x = f32::from(position.x - bounds.origin.x) / scale;
    let y = f32::from(position.y - bounds.origin.y) / scale;
    (x <= props.width as f32 && y <= props.height as f32).then_some((x, y))
}

fn canvas_props(props: &ui_ext::v0_2::Props) -> Option<&CanvasProps> {
    match props {
        ui_ext::v0_2::Props::Canvas(canvas) => Some(canvas),
        _ => None,
    }
}

/// The element for a canvas node, from the tree's shared props (never copied per frame).
/// `on_click` gets canvas coordinates; `None` when it is not clickable. With a `slot`, a long
/// draw list is painted through a cached texture (`canvas_texture`).
pub(crate) fn canvas_element(
    element_id: gpui::ElementId,
    shared: Arc<ui_ext::v0_2::Props>,
    slot: Option<crate::canvas_texture::CanvasSlot>,
    on_click: Option<Rc<dyn Fn(f32, f32, &mut Window, &mut App)>>,
) -> gpui::AnyElement {
    let Some(props) = canvas_props(&shared) else {
        return gpui::div().into_any_element();
    };
    let painted: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::new(Cell::new(None));
    let (width, height) = (props.width as f32, props.height as f32);
    let description: SharedString = props.description.clone().into();
    let paint_props = shared.clone();
    let record = painted.clone();
    let surface = gpui_canvas(
        move |bounds, _, _| record.set(Some(bounds)),
        move |bounds, (), window, cx| {
            if let Some(props) = canvas_props(&paint_props) {
                crate::canvas_texture::paint(slot.as_ref(), &paint_props, props, bounds, window, cx)
            }
        },
    );
    let surface = match props.fit {
        CanvasFit::Clip => surface.w(px(width)).h(px(height)).flex_none(),
        CanvasFit::ScaleDown => surface
            .w_full()
            .max_w(px(width))
            .aspect_ratio(width / height),
    };
    gpui::div()
        .id(element_id)
        .aria_label(description.clone())
        .tooltip(ui::Tooltip::text(description))
        .max_w_full()
        .overflow_hidden()
        .when_some(on_click, |this, on_click| {
            let click_props = shared.clone();
            this.cursor_pointer().on_mouse_down(
                gpui::MouseButton::Left,
                move |event, window, cx| {
                    if let Some(bounds) = painted.get()
                        && let Some(props) = canvas_props(&click_props)
                        && let Some((x, y)) = to_canvas(props, bounds, event.position)
                    {
                        cx.stop_propagation();
                        on_click(x, y, window, cx);
                    }
                },
            )
        })
        .child(surface)
        .into_any_element()
}

pub(crate) fn paint_ops(
    props: &CanvasProps,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let scale = fit_scale(props, bounds.size.width);
    let at = |x: f32, y: f32| -> Point<Pixels> {
        point(
            bounds.origin.x + px(x * scale),
            bounds.origin.y + px(y * scale),
        )
    };
    let length = |value: f32| px(value * scale);
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        for op in &props.ops {
            match op {
                DrawOp::Line { from, to, stroke } => {
                    let mut path = PathBuilder::stroke(length(stroke.width.0));
                    path.move_to(at(from.x.0, from.y.0));
                    path.line_to(at(to.x.0, to.y.0));
                    if let Ok(path) = path.build() {
                        window.paint_path(path, paint_color(stroke.paint, cx));
                    }
                }
                DrawOp::Rect {
                    bounds: rect,
                    fill,
                    stroke,
                } => paint_box(window, cx, &at, &length, rect, 0., *fill, *stroke),
                DrawOp::RoundedRect {
                    bounds: rect,
                    radius,
                    fill,
                    stroke,
                } => {
                    let radius = radius
                        .0
                        .min(rect.width.0.abs() / 2.)
                        .min(rect.height.0.abs() / 2.)
                        .max(0.);
                    paint_box(window, cx, &at, &length, rect, radius, *fill, *stroke)
                }
                DrawOp::Circle {
                    center,
                    radius,
                    fill,
                    stroke,
                } => {
                    let r = radius.0.max(0.);
                    let rect = ui_ext::canvas::Rect {
                        x: (center.x.0 - r).into(),
                        y: (center.y.0 - r).into(),
                        width: (2. * r).into(),
                        height: (2. * r).into(),
                    };
                    paint_box(window, cx, &at, &length, &rect, r, *fill, *stroke)
                }
                DrawOp::Path {
                    segments,
                    fill,
                    stroke,
                } => {
                    let build = |mut builder: PathBuilder| {
                        for segment in segments {
                            match segment {
                                PathSegment::MoveTo(p) => builder.move_to(at(p.x.0, p.y.0)),
                                PathSegment::LineTo(p) => builder.line_to(at(p.x.0, p.y.0)),
                                PathSegment::QuadTo(control, end) => builder
                                    .curve_to(at(end.x.0, end.y.0), at(control.x.0, control.y.0)),
                                PathSegment::CubicTo(a, b, end) => builder.cubic_bezier_to(
                                    at(end.x.0, end.y.0),
                                    at(a.x.0, a.y.0),
                                    at(b.x.0, b.y.0),
                                ),
                                PathSegment::Close => builder.close(),
                            }
                        }
                        builder.build().ok()
                    };
                    if let Some(fill) = fill
                        && let Some(path) = build(PathBuilder::fill())
                    {
                        window.paint_path(path, paint_color(*fill, cx));
                    }
                    if let Some(stroke) = stroke
                        && let Some(path) = build(PathBuilder::stroke(length(stroke.width.0)))
                    {
                        window.paint_path(path, paint_color(stroke.paint, cx));
                    }
                }
                DrawOp::Text {
                    at: origin,
                    text,
                    paint,
                    size,
                    anchor,
                    monospace,
                } => {
                    let settings = theme::theme_settings(cx);
                    let base = settings.ui_font_size(cx);
                    let font = if *monospace {
                        settings.buffer_font(cx).clone()
                    } else {
                        settings.ui_font(cx).clone()
                    };
                    let factor = match size {
                        CanvasTextSize::Small => 0.85,
                        CanvasTextSize::Medium => 1.,
                        CanvasTextSize::Large => 1.2,
                    };
                    let font_size = base * factor * scale;
                    let run = TextRun {
                        len: text.len(),
                        font,
                        color: paint_color(*paint, cx),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    };
                    let line = window.text_system().shape_line(
                        text.clone().into(),
                        font_size,
                        &[run],
                        None,
                    );
                    let anchor_x = match anchor {
                        TextAnchor::Start => px(0.),
                        TextAnchor::Middle => line.width / 2.,
                        TextAnchor::End => line.width,
                    };
                    let baseline = at(origin.x.0, origin.y.0);
                    let top_left = point(baseline.x - anchor_x, baseline.y - line.ascent);
                    line.paint(top_left, font_size * 1.3, TextAlign::Left, None, window, cx)
                        .ok();
                }
            }
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn paint_box(
    window: &mut Window,
    cx: &App,
    at: &impl Fn(f32, f32) -> Point<Pixels>,
    length: &impl Fn(f32) -> Pixels,
    rect: &ui_ext::canvas::Rect,
    radius: f32,
    fill: Option<Paint>,
    stroke: Option<ui_ext::canvas::Stroke>,
) {
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
    let bounds = Bounds::new(at(x, y), gpui::size(length(width), length(height)));
    let background = fill.map_or(transparent_black(), |fill| paint_color(fill, cx));
    let (border, border_color) = match stroke {
        Some(stroke) => (length(stroke.width.0), paint_color(stroke.paint, cx)),
        None => (px(0.), transparent_black()),
    };
    window.paint_quad(quad(
        bounds,
        Corners::all(length(radius)),
        background,
        Edges::all(border),
        border_color,
        BorderStyle::Solid,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use ui_ext::canvas::{CanvasProps, Num};

    fn props(fit: CanvasFit) -> CanvasProps {
        CanvasProps {
            width: 200,
            height: 100,
            description: "chart".into(),
            ops: Vec::new(),
            clickable: true,
            fit,
        }
    }

    /// §13 hit-test mapping: back through `scale-down` and the scroll (the painted box's origin).
    #[test]
    fn test_clicks_map_to_canvas_coordinates() {
        let clip = props(CanvasFit::Clip);
        // Scrolled: the box is painted 300 px higher than its layout position.
        let bounds = Bounds::new(point(px(10.), px(-290.)), gpui::size(px(200.), px(100.)));
        assert_eq!(
            to_canvas(&clip, bounds, point(px(60.), px(-240.))),
            Some((50., 50.))
        );
        assert_eq!(
            to_canvas(&clip, bounds, point(px(9.), px(-240.))),
            None,
            "outside"
        );

        // scale-down to half the width.
        let scaled = props(CanvasFit::ScaleDown);
        let bounds = Bounds::new(point(px(0.), px(0.)), gpui::size(px(100.), px(50.)));
        assert_eq!(fit_scale(&scaled, px(100.)), 0.5);
        assert_eq!(
            to_canvas(&scaled, bounds, point(px(50.), px(25.))),
            Some((100., 50.))
        );
        // Never grows.
        assert_eq!(fit_scale(&scaled, px(400.)), 1.);
        let _ = Num(0.);
    }

    #[test]
    fn test_data_colors_differ_and_follow_the_theme() {
        let series = [
            DataColor::Series1,
            DataColor::Series2,
            DataColor::Series3,
            DataColor::Series4,
            DataColor::Series5,
            DataColor::Series6,
            DataColor::Series7,
            DataColor::Series8,
        ];
        for appearance in [Appearance::Light, Appearance::Dark] {
            let colors: Vec<Hsla> = series
                .iter()
                .map(|series| data_color(*series, appearance))
                .collect();
            for (i, a) in colors.iter().enumerate() {
                for b in &colors[i + 1..] {
                    assert_ne!(a, b);
                }
            }
        }
        assert!(
            data_color(DataColor::Series1, Appearance::Dark).l
                > data_color(DataColor::Series1, Appearance::Light).l
        );
    }
}

/// Plan 4.14's paint bench (Q14): one 16384-op canvas painted frame after frame, directly and
/// through its texture (`canvas_texture`), and the raster of one update on its own. Ignored by default; meaningful
/// only in an optimized build:
/// `cargo test --profile release-fast -p ui_ext_host --lib canvas_paint_bench -- --ignored --nocapture`.
///
/// It times `paint_ops` itself (inside a real paint phase of a test window: quads, path
/// tessellation, text lines) and the whole frame around it. The test platform's text system
/// shapes without a font and rasterizes no glyphs, so the text figures are a lower bound; GPU
/// time is not part of it.
#[cfg(test)]
mod bench {
    use std::{cell::RefCell, time::Duration, time::Instant};

    use gpui::{AvailableSpace, TestAppContext, size};
    use ui_ext::canvas::{CanvasColor, Num, Paint, Point as CanvasPoint, Rect, Stroke};

    use super::*;

    const OPS: usize = ui_ext::canvas::MAX_OPS;
    const SIDE: u32 = 1024;
    const FRAMES: usize = 40;
    const WARMUP: usize = 5;

    fn p(x: f32, y: f32) -> CanvasPoint {
        CanvasPoint {
            x: Num(x),
            y: Num(y),
        }
    }

    /// A 128 × 128 grid over the 1024 × 1024 box, so every op is inside it.
    fn spot(i: usize) -> (f32, f32) {
        ((i % 128) as f32 * 8., (i / 128 % 128) as f32 * 8.)
    }

    fn paint(i: usize) -> Paint {
        let series = [
            DataColor::Series1,
            DataColor::Series2,
            DataColor::Series3,
            DataColor::Series4,
            DataColor::Series5,
            DataColor::Series6,
            DataColor::Series7,
            DataColor::Series8,
        ];
        Paint {
            color: CanvasColor::Data(series[i % 8]),
            opacity: 90,
        }
    }

    fn stroke(i: usize) -> Stroke {
        Stroke {
            paint: paint(i + 3),
            width: Num(1.),
        }
    }

    fn rect(i: usize) -> DrawOp {
        let (x, y) = spot(i);
        DrawOp::Rect {
            bounds: Rect {
                x: Num(x),
                y: Num(y),
                width: Num(7.),
                height: Num(7.),
            },
            fill: Some(paint(i)),
            stroke: Some(stroke(i)),
        }
    }

    fn rounded(i: usize) -> DrawOp {
        let (x, y) = spot(i);
        DrawOp::RoundedRect {
            bounds: Rect {
                x: Num(x),
                y: Num(y),
                width: Num(7.),
                height: Num(5.),
            },
            radius: Num(2.),
            fill: Some(paint(i)),
            stroke: None,
        }
    }

    fn circle(i: usize) -> DrawOp {
        let (x, y) = spot(i);
        DrawOp::Circle {
            center: p(x + 4., y + 4.),
            radius: Num(3.5),
            fill: Some(paint(i)),
            stroke: Some(stroke(i)),
        }
    }

    fn line(i: usize) -> DrawOp {
        let (x, y) = spot(i);
        DrawOp::Line {
            from: p(x, y),
            to: p(x + 7., y + 5.),
            stroke: stroke(i),
        }
    }

    /// Four segments: 16384 of them is the 65536-segment cap.
    fn path(i: usize) -> DrawOp {
        let (x, y) = spot(i);
        DrawOp::Path {
            segments: vec![
                PathSegment::MoveTo(p(x, y + 7.)),
                PathSegment::LineTo(p(x + 3., y)),
                PathSegment::CubicTo(p(x + 5., y), p(x + 7., y + 3.), p(x + 7., y + 7.)),
                PathSegment::Close,
            ],
            fill: Some(paint(i)),
            stroke: Some(stroke(i)),
        }
    }

    fn text(i: usize) -> DrawOp {
        let (x, y) = spot(i);
        DrawOp::Text {
            at: p(x, y + 7.),
            text: format!("{}", i % 1000),
            paint: paint(i),
            size: CanvasTextSize::Small,
            anchor: TextAnchor::Start,
            monospace: false,
        }
    }

    fn mixed(i: usize) -> DrawOp {
        match i % 6 {
            0 => rect(i),
            1 => circle(i),
            2 => line(i),
            3 => path(i),
            4 => text(i),
            _ => rounded(i),
        }
    }

    fn canvas(op: fn(usize) -> DrawOp) -> Arc<ui_ext::v0_2::Props> {
        Arc::new(ui_ext::v0_2::Props::Canvas(CanvasProps {
            width: SIDE,
            height: SIDE,
            description: "bench".into(),
            ops: (0..OPS).map(op).collect(),
            clickable: false,
            fit: CanvasFit::Clip,
        }))
    }

    struct Stats {
        median: Duration,
        p95: Duration,
        max: Duration,
    }

    fn stats(mut samples: Vec<Duration>) -> Stats {
        samples.sort();
        Stats {
            median: samples[samples.len() / 2],
            p95: samples[samples.len() * 95 / 100],
            max: samples[samples.len() - 1],
        }
    }

    /// GPUI's own cost for as many quads, with nothing of ours: the floor for quad ops.
    fn raw_quads(bounds: Bounds<Pixels>, window: &mut Window) {
        let color = gpui::red();
        for i in 0..OPS {
            let (x, y) = spot(i);
            window.paint_quad(gpui::fill(
                Bounds::new(
                    point(bounds.origin.x + px(x), bounds.origin.y + px(y)),
                    size(px(7.), px(7.)),
                ),
                color,
            ));
        }
    }

    fn ms(duration: Duration) -> f64 {
        duration.as_secs_f64() * 1000.
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Mode {
        Baseline,
        Direct,
        Texture,
    }

    /// The paint and frame times of `FRAMES` frames of one canvas.
    fn measure(
        vcx: &mut gpui::VisualTestContext,
        props: &Arc<ui_ext::v0_2::Props>,
        mode: Mode,
    ) -> (Stats, Stats) {
        let slot = crate::canvas_texture::CanvasSlot {
            key: crate::canvas_texture::SlotKey { view: 0, node: 0 },
            extension_id: "bench".into(),
        };
        let paints = Rc::new(RefCell::new(Vec::new()));
        let mut frames = Vec::new();
        for frame in 0..WARMUP + FRAMES {
            let (props, paints_in, slot) = (props.clone(), paints.clone(), slot.clone());
            let start = Instant::now();
            vcx.draw(
                point(px(0.), px(0.)),
                size(
                    AvailableSpace::Definite(px(SIDE as f32)),
                    AvailableSpace::Definite(px(SIDE as f32)),
                ),
                move |_, _| {
                    gpui_canvas(
                        |_, _, _| (),
                        move |bounds, (), window, cx| {
                            let Some(canvas) = canvas_props(&props) else {
                                return;
                            };
                            let start = Instant::now();
                            match mode {
                                Mode::Baseline => raw_quads(bounds, window),
                                Mode::Direct => paint_ops(canvas, bounds, window, cx),
                                Mode::Texture => crate::canvas_texture::paint(
                                    Some(&slot),
                                    &props,
                                    canvas,
                                    bounds,
                                    window,
                                    cx,
                                ),
                            }
                            paints_in.borrow_mut().push(start.elapsed());
                        },
                    )
                    .w(px(SIDE as f32))
                    .h(px(SIDE as f32))
                },
            );
            let elapsed = start.elapsed();
            // A real frame in between, so each measured paint starts on an empty scene (and the
            // warm-up's raster finishes).
            vcx.run_until_parked();
            if frame >= WARMUP {
                frames.push(elapsed);
            }
        }
        let paints = stats(paints.borrow_mut().split_off(WARMUP));
        (paints, stats(frames))
    }

    #[gpui::test]
    #[ignore = "bench: run in release-fast with --ignored --nocapture"]
    fn canvas_paint_bench(cx: &mut TestAppContext) {
        crate::tests::init_test(cx);
        let cases: [(&str, fn(usize) -> DrawOp); 8] = [
            ("gpui paint_quad alone (baseline)", rect),
            ("rect (fill + stroke)", rect),
            ("rounded rect (fill)", rounded),
            ("circle (fill + stroke)", circle),
            ("line", line),
            ("path, 4 segments (fill + stroke)", path),
            ("text (no glyphs)", text),
            ("mixed, 1/6 of each", mixed),
        ];
        let vcx = cx.add_empty_window();
        let scale = vcx.update(|window, _| window.scale_factor());
        let device = (
            (SIDE as f32 * scale).round() as u32,
            (SIDE as f32 * scale).round() as u32,
        );
        println!(
            "canvas paint bench: {OPS} ops, {SIDE} x {SIDE} at {scale}x, {FRAMES} frames after {WARMUP}"
        );
        println!(
            "{:<34} {:>11} {:>8} {:>8} {:>13} {:>13} {:>13} {:>13}",
            "case", "paint med", "p95", "max", "frame med", "tex paint", "tex frame", "raster med"
        );
        for (index, (name, op)) in cases.into_iter().enumerate() {
            let props = canvas(op);
            if index == 0 {
                let (paints, frames) = measure(vcx, &props, Mode::Baseline);
                println!(
                    "{:<34} {:>8.2} ms {:>5.2} ms {:>5.2} ms {:>10.2} ms",
                    name,
                    ms(paints.median),
                    ms(paints.p95),
                    ms(paints.max),
                    ms(frames.median),
                );
                continue;
            }
            let (paints, frames) = measure(vcx, &props, Mode::Direct);
            let (texture_paints, texture_frames) = measure(vcx, &props, Mode::Texture);
            assert!(
                vcx.update(|_, cx| crate::canvas_texture::drawn_from_for_bench(&props, cx)),
                "the texture case drew its texture"
            );
            // One update's raster on its own (main-thread colour lookup, SVG, resvg), as the
            // background task does it.
            let canvas = canvas_props(&props).unwrap();
            let fonts = vcx.update(|_, cx| crate::images::shared_fonts(cx));
            let rasters = stats(
                (0..5)
                    .map(|_| {
                        let start = Instant::now();
                        let palette = vcx.update(|_, cx| {
                            crate::canvas_texture::Palette::resolve(canvas, 14., cx)
                        });
                        let svg = crate::canvas_texture::to_svg(canvas, &palette);
                        crate::canvas_texture::rasterize(&svg, device, fonts.clone()).unwrap();
                        start.elapsed()
                    })
                    .collect(),
            );
            println!(
                "{:<34} {:>8.2} ms {:>5.2} ms {:>5.2} ms {:>10.2} ms {:>10.3} ms {:>10.2} ms {:>10.1} ms",
                name,
                ms(paints.median),
                ms(paints.p95),
                ms(paints.max),
                ms(frames.median),
                ms(texture_paints.median),
                ms(texture_frames.median),
                ms(rasters.median),
            );
        }
    }
}
