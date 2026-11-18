//! `fyx:ui` 0.3 canvases (design 0.3, section 13; plan 4.15): builders for the draw list of a
//! `canvas` node and a hit-test helper, since hit-testing a click is the extension's job.
//!
//! ```ignore
//! let mut draw = DrawList::new();
//! for (index, value) in values.iter().enumerate() {
//!     draw.rect(bar(index, *value), Some(Paint::data(DataColor::cycle(index))), None);
//! }
//! let canvas = CanvasProps::new(240, 120, "Commits per day").draw(draw).clickable();
//! ```
//!
//! The values are the generated WIT records (`Point`, `Rect`, `Paint`, `Stroke`, `DrawOp`, …);
//! these are constructors and builders on top. Nothing here checks the caps: [`SurfaceTree`]
//! does, with the host's own rules (`ui_ext::canvas`), when a canvas goes into a tree.
//!
//! [`SurfaceTree`]: crate::SurfaceTree

use crate::{
    CanvasColor, CanvasFit, CanvasProps, CanvasTextSize, CircleOp, ColorToken, DataColor, DrawOp,
    LineOp, Paint, PathOp, PathSegment, Point, Props, Rect, RectOp, RoundedRectOp, Stroke,
    TextAnchor, TextOp,
};

/// A point in canvas coordinates: logical pixels from the top-left corner.
pub fn pt(x: f32, y: f32) -> Point {
    Point { x, y }
}

impl Point {
    pub fn new(x: f32, y: f32) -> Self {
        pt(x, y)
    }
}

impl Rect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Whether `(x, y)` lies inside, edges included.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x <= self.x + self.width && y >= self.y && y <= self.y + self.height
    }
}

impl Paint {
    /// A theme colour, opaque.
    pub fn token(color: ColorToken) -> Self {
        Self {
            color: CanvasColor::Token(color),
            opacity: 100,
        }
    }

    /// A data series colour, opaque.
    pub fn data(color: DataColor) -> Self {
        Self {
            color: CanvasColor::Data(color),
            opacity: 100,
        }
    }

    /// Percent, 0 to 100.
    pub fn opacity(mut self, percent: u8) -> Self {
        self.opacity = percent;
        self
    }
}

impl Stroke {
    /// `width` in logical pixels, 0.5 to 16.
    pub fn new(paint: Paint, width: f32) -> Self {
        Self { paint, width }
    }
}

impl DataColor {
    /// The eight series colours in order, then again from the first: the colour of series
    /// `index`.
    pub fn cycle(index: usize) -> Self {
        const ALL: [DataColor; 8] = [
            DataColor::Series1,
            DataColor::Series2,
            DataColor::Series3,
            DataColor::Series4,
            DataColor::Series5,
            DataColor::Series6,
            DataColor::Series7,
            DataColor::Series8,
        ];
        ALL[index % ALL.len()]
    }
}

/// How a text op is set: small, at the start, in the UI font by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextStyle {
    pub size: CanvasTextSize,
    pub anchor: TextAnchor,
    pub monospace: bool,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            size: CanvasTextSize::Small,
            anchor: TextAnchor::Start,
            monospace: false,
        }
    }
}

impl TextStyle {
    pub fn size(mut self, size: CanvasTextSize) -> Self {
        self.size = size;
        self
    }

    pub fn anchor(mut self, anchor: TextAnchor) -> Self {
        self.anchor = anchor;
        self
    }

    pub fn monospace(mut self) -> Self {
        self.monospace = true;
        self
    }
}

/// A path for [`DrawList::path`]: start with [`Path::move_to`], and give it a fill (non-zero
/// rule), a stroke or both.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path {
    segments: Vec<PathSegment>,
    fill: Option<Paint>,
    stroke: Option<Stroke>,
}

impl Path {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn move_to(mut self, to: Point) -> Self {
        self.segments.push(PathSegment::MoveTo(to));
        self
    }

    pub fn line_to(mut self, to: Point) -> Self {
        self.segments.push(PathSegment::LineTo(to));
        self
    }

    pub fn quad_to(mut self, control: Point, to: Point) -> Self {
        self.segments.push(PathSegment::QuadTo((control, to)));
        self
    }

    pub fn cubic_to(mut self, first: Point, second: Point, to: Point) -> Self {
        self.segments
            .push(PathSegment::CubicTo((first, second, to)));
        self
    }

    pub fn close(mut self) -> Self {
        self.segments.push(PathSegment::Close);
        self
    }

    pub fn fill(mut self, paint: Paint) -> Self {
        self.fill = Some(paint);
        self
    }

    pub fn stroke(mut self, stroke: Stroke) -> Self {
        self.stroke = Some(stroke);
        self
    }

    /// A polyline through `points`, for sparklines and line charts.
    pub fn polyline(points: impl IntoIterator<Item = Point>) -> Self {
        let mut path = Self::new();
        for (index, point) in points.into_iter().enumerate() {
            path.segments.push(if index == 0 {
                PathSegment::MoveTo(point)
            } else {
                PathSegment::LineTo(point)
            });
        }
        path
    }
}

/// The ops of a canvas, drawn in order, later over earlier. A `&mut` builder, since charts are
/// built in loops.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrawList {
    ops: Vec<DrawOp>,
}

impl DrawList {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, op: DrawOp) -> &mut Self {
        self.ops.push(op);
        self
    }

    pub fn line(&mut self, from: Point, to: Point, stroke: Stroke) -> &mut Self {
        self.push(DrawOp::Line(LineOp { from, to, stroke }))
    }

    pub fn rect(&mut self, bounds: Rect, fill: Option<Paint>, stroke: Option<Stroke>) -> &mut Self {
        self.push(DrawOp::Rect(RectOp {
            bounds,
            fill,
            stroke,
        }))
    }

    /// `radius` is clamped by the host to half the shorter side.
    pub fn rounded_rect(
        &mut self,
        bounds: Rect,
        radius: f32,
        fill: Option<Paint>,
        stroke: Option<Stroke>,
    ) -> &mut Self {
        self.push(DrawOp::RoundedRect(RoundedRectOp {
            bounds,
            radius,
            fill,
            stroke,
        }))
    }

    pub fn circle(
        &mut self,
        center: Point,
        radius: f32,
        fill: Option<Paint>,
        stroke: Option<Stroke>,
    ) -> &mut Self {
        self.push(DrawOp::Circle(CircleOp {
            center,
            radius,
            fill,
            stroke,
        }))
    }

    pub fn path(&mut self, path: Path) -> &mut Self {
        self.push(DrawOp::Path(PathOp {
            segments: path.segments,
            fill: path.fill,
            stroke: path.stroke,
        }))
    }

    /// One line of text, at most 256 bytes, in the [`TextStyle::default`] style; `at` is the
    /// baseline at its start. The extension cannot measure text: place labels with the anchor.
    pub fn text(&mut self, at: Point, text: impl Into<String>, paint: Paint) -> &mut Self {
        self.text_with(at, text, paint, TextStyle::default())
    }

    pub fn text_with(
        &mut self,
        at: Point,
        text: impl Into<String>,
        paint: Paint,
        style: TextStyle,
    ) -> &mut Self {
        self.push(DrawOp::Text(TextOp {
            at,
            text: text.into(),
            paint,
            size: style.size,
            anchor: style.anchor,
            monospace: style.monospace,
        }))
    }

    /// Ops so far; the cap is 16384 per canvas.
    pub fn len(&self) -> usize {
        self.ops.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Path segments of all paths so far; the cap is 65536 per canvas.
    pub fn segments(&self) -> usize {
        self.ops
            .iter()
            .map(|op| match op {
                DrawOp::Path(path) => path.segments.len(),
                _ => 0,
            })
            .sum()
    }

    /// Bytes of text of all text ops so far; the cap is 256 KiB per canvas.
    pub fn text_bytes(&self) -> usize {
        self.ops
            .iter()
            .map(|op| match op {
                DrawOp::Text(text) => text.text.len(),
                _ => 0,
            })
            .sum()
    }

    pub fn ops(&self) -> &[DrawOp] {
        &self.ops
    }

    pub fn into_ops(self) -> Vec<DrawOp> {
        self.ops
    }
}

impl CanvasProps {
    /// An empty, not clickable canvas of `width` × `height` logical pixels (1 to 4096), drawn at
    /// its size and clipped (`CanvasFit::Clip`). `description` (1 to 1024 bytes) is required:
    /// screen readers read it, it is the tooltip, and the host shows it in place of the drawing
    /// when the canvas cannot be drawn.
    pub fn new(width: u32, height: u32, description: impl Into<String>) -> Self {
        Self {
            width,
            height,
            description: description.into(),
            ops: Vec::new(),
            clickable: false,
            fit: CanvasFit::Clip,
        }
    }

    pub fn draw(mut self, draw: DrawList) -> Self {
        self.ops = draw.into_ops();
        self
    }

    /// Clicks send `UiEvent::CanvasClick` with canvas coordinates; a click is a gesture, like a
    /// button click.
    pub fn clickable(mut self) -> Self {
        self.clickable = true;
        self
    }

    pub fn fit(mut self, fit: CanvasFit) -> Self {
        self.fit = fit;
        self
    }
}

impl From<CanvasProps> for Props {
    fn from(props: CanvasProps) -> Self {
        Props::Canvas(props)
    }
}

/// Regions of a canvas and what each stands for, to turn a `canvas-click` into a value. Built
/// next to the draw list; the region added last wins where regions overlap, as the op drawn
/// last is on top.
#[derive(Clone, Debug, PartialEq)]
pub struct CanvasHits<V> {
    regions: Vec<(Rect, V)>,
}

impl<V> Default for CanvasHits<V> {
    fn default() -> Self {
        Self {
            regions: Vec::new(),
        }
    }
}

impl<V> CanvasHits<V> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, region: Rect, value: V) -> &mut Self {
        self.regions.push((region, value));
        self
    }

    /// The topmost region's value at `(x, y)`, if any.
    pub fn at(&self, x: f32, y: f32) -> Option<&V> {
        self.regions
            .iter()
            .rev()
            .find(|(region, _)| region.contains(x, y))
            .map(|(_, value)| value)
    }

    pub fn clear(&mut self) {
        self.regions.clear();
    }

    pub fn len(&self) -> usize {
        self.regions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }
}

/// The SDK's draw list in the host's model, for `ui_ext::canvas`'s checks.
pub(crate) fn host_canvas(props: &CanvasProps) -> ui_ext::canvas::CanvasProps {
    use ui_ext::canvas as host;
    let point = |point: &Point| host::Point {
        x: point.x.into(),
        y: point.y.into(),
    };
    let paint = |paint: &Paint| host::Paint {
        color: match paint.color {
            CanvasColor::Token(token) => host::CanvasColor::Token(token.into()),
            CanvasColor::Data(data) => host::CanvasColor::Data(match data {
                DataColor::Series1 => host::DataColor::Series1,
                DataColor::Series2 => host::DataColor::Series2,
                DataColor::Series3 => host::DataColor::Series3,
                DataColor::Series4 => host::DataColor::Series4,
                DataColor::Series5 => host::DataColor::Series5,
                DataColor::Series6 => host::DataColor::Series6,
                DataColor::Series7 => host::DataColor::Series7,
                DataColor::Series8 => host::DataColor::Series8,
            }),
        },
        opacity: paint.opacity,
    };
    let stroke = |stroke: &Stroke| host::Stroke {
        paint: paint(&stroke.paint),
        width: stroke.width.into(),
    };
    let rect = |rect: &Rect| host::Rect {
        x: rect.x.into(),
        y: rect.y.into(),
        width: rect.width.into(),
        height: rect.height.into(),
    };
    host::CanvasProps {
        width: props.width,
        height: props.height,
        description: props.description.clone(),
        ops: props
            .ops
            .iter()
            .map(|op| match op {
                DrawOp::Line(line) => host::DrawOp::Line {
                    from: point(&line.from),
                    to: point(&line.to),
                    stroke: stroke(&line.stroke),
                },
                DrawOp::Rect(op) => host::DrawOp::Rect {
                    bounds: rect(&op.bounds),
                    fill: op.fill.as_ref().map(paint),
                    stroke: op.stroke.as_ref().map(stroke),
                },
                DrawOp::RoundedRect(op) => host::DrawOp::RoundedRect {
                    bounds: rect(&op.bounds),
                    radius: op.radius.into(),
                    fill: op.fill.as_ref().map(paint),
                    stroke: op.stroke.as_ref().map(stroke),
                },
                DrawOp::Circle(op) => host::DrawOp::Circle {
                    center: point(&op.center),
                    radius: op.radius.into(),
                    fill: op.fill.as_ref().map(paint),
                    stroke: op.stroke.as_ref().map(stroke),
                },
                DrawOp::Path(op) => host::DrawOp::Path {
                    segments: op
                        .segments
                        .iter()
                        .map(|segment| match segment {
                            PathSegment::MoveTo(to) => host::PathSegment::MoveTo(point(to)),
                            PathSegment::LineTo(to) => host::PathSegment::LineTo(point(to)),
                            PathSegment::QuadTo((control, to)) => {
                                host::PathSegment::QuadTo(point(control), point(to))
                            }
                            PathSegment::CubicTo((first, second, to)) => {
                                host::PathSegment::CubicTo(point(first), point(second), point(to))
                            }
                            PathSegment::Close => host::PathSegment::Close,
                        })
                        .collect(),
                    fill: op.fill.as_ref().map(paint),
                    stroke: op.stroke.as_ref().map(stroke),
                },
                DrawOp::Text(op) => host::DrawOp::Text {
                    at: point(&op.at),
                    text: op.text.clone(),
                    paint: paint(&op.paint),
                    size: match op.size {
                        CanvasTextSize::Small => host::CanvasTextSize::Small,
                        CanvasTextSize::Medium => host::CanvasTextSize::Medium,
                        CanvasTextSize::Large => host::CanvasTextSize::Large,
                    },
                    anchor: match op.anchor {
                        TextAnchor::Start => host::TextAnchor::Start,
                        TextAnchor::Middle => host::TextAnchor::Middle,
                        TextAnchor::End => host::TextAnchor::End,
                    },
                    monospace: op.monospace,
                },
            })
            .collect(),
        clickable: props.clickable,
        fit: match props.fit {
            CanvasFit::Clip => host::CanvasFit::Clip,
            CanvasFit::ScaleDown => host::CanvasFit::ScaleDown,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SurfaceTree, TreeError, list_row};

    fn chart() -> CanvasProps {
        let mut draw = DrawList::new();
        for index in 0..3 {
            draw.rect(
                Rect::new(index as f32 * 20.0, 10.0, 16.0, 50.0),
                Some(Paint::data(DataColor::cycle(index))),
                None,
            );
        }
        draw.line(
            pt(0.0, 60.0),
            pt(60.0, 60.0),
            Stroke::new(Paint::token(ColorToken::Border), 1.0),
        )
        .path(
            Path::polyline([pt(0.0, 50.0), pt(30.0, 20.0), pt(60.0, 40.0)]).stroke(Stroke::new(
                Paint::token(ColorToken::TextAccent).opacity(80),
                1.5,
            )),
        )
        .text_with(
            pt(30.0, 8.0),
            "Commits",
            Paint::token(ColorToken::TextMuted),
            TextStyle::default().anchor(TextAnchor::Middle),
        );
        assert_eq!((draw.len(), draw.segments(), draw.text_bytes()), (6, 3, 7));
        CanvasProps::new(64, 64, "Three bars")
            .draw(draw)
            .clickable()
    }

    #[test]
    fn builders_make_the_wit_values() {
        let canvas = chart();
        assert!(canvas.clickable);
        assert_eq!(canvas.fit, CanvasFit::Clip);
        assert!(matches!(
            &canvas.ops[1],
            DrawOp::Rect(RectOp {
                fill: Some(Paint {
                    color: CanvasColor::Data(DataColor::Series2),
                    opacity: 100
                }),
                ..
            })
        ));
        assert_eq!(DataColor::cycle(9), DataColor::Series2);
        ui_ext::canvas::check_canvas_props(&host_canvas(&canvas)).expect("the host accepts it");
    }

    #[test]
    fn hits_find_the_topmost_region() {
        let mut hits = CanvasHits::new();
        hits.add(Rect::new(0.0, 0.0, 100.0, 100.0), "background")
            .add(Rect::new(10.0, 10.0, 20.0, 20.0), "bar");
        assert_eq!(hits.at(15.0, 15.0), Some(&"bar"));
        assert_eq!(hits.at(50.0, 50.0), Some(&"background"));
        assert_eq!(hits.at(150.0, 50.0), None);
    }

    #[test]
    fn the_surface_tree_checks_canvases_as_the_host_does() {
        let mut surface = SurfaceTree::default();
        let mut bad = chart();
        bad.ops.push(DrawOp::Line(LineOp {
            from: pt(f32::NAN, 0.0),
            to: pt(1.0, 1.0),
            stroke: Stroke::new(Paint::token(ColorToken::Text), 1.0),
        }));
        let error = surface
            .mount(|tree| {
                tree.canvas(bad.clone());
            })
            .unwrap_err();
        assert!(
            matches!(error, TreeError::InvalidCanvas { node: 1, .. }),
            "{error}"
        );
        assert!(!surface.is_mounted());

        let mut column = 0;
        surface
            .mount(|tree| {
                column = tree.column(|tree| {
                    for _ in 0..16 {
                        tree.canvas(chart());
                    }
                });
            })
            .expect("16 canvases");
        let error = surface
            .insert(column, None, |tree| {
                tree.canvas(chart());
            })
            .unwrap_err();
        assert_eq!(error, TreeError::TooManyCanvases { count: 17, max: 16 });
        let first = surface.children(column)[0];
        assert!(matches!(
            surface.set_props(first, bad),
            Err(TreeError::InvalidCanvas { .. })
        ));
        surface
            .rebuild(first, |tree| {
                tree.canvas(chart());
            })
            .expect("replacing a canvas keeps 16");

        let error = list_row(0, |tree| {
            tree.canvas(chart());
        })
        .unwrap_err();
        assert_eq!(error, TreeError::CanvasInRow);
    }
}
