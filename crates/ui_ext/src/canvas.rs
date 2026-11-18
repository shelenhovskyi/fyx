//! Canvases of `fyx:ui@0.3.0` (docs/fyx/wit-0.3/design.md, section 13): the `canvas` node's draw
//! list, the limits the host, the renderer, the harness and the SDK share, and their checks.
//!
//! [`check_canvas_props`] is the props rule (sizes, caps, finite coordinates within
//! ±[`MAX_COORDINATE`], strokes, text, path shape) and [`check_canvas_placement`] the surface
//! rule (panels only). The two tree rules, at most [`MAX_CANVASES_PER_SURFACE`] per surface and
//! none in list rows, are enforced by [`crate::v0_2::tree::Tree`] itself.

use serde::{Deserialize, Serialize};

use crate::types::ColorToken;
use crate::v0_2::{HostErrorCode, NodeId, SurfaceKind};

/// The manifest capability for `canvas` nodes (tier T0).
pub const CAPABILITY_CANVAS: &str = "ui:canvas";
/// Each side of a canvas, in logical pixels.
pub const MAX_CANVAS_SIDE: u32 = 4096;
pub const MAX_DESCRIPTION_BYTES: usize = 1024;
pub const MAX_OPS: usize = 16384;
/// Path segments of all the paths of one canvas together.
pub const MAX_PATH_SEGMENTS: usize = 65536;
/// Bytes of text of all the text ops of one canvas together.
pub const MAX_TEXT_BYTES: usize = 256 * 1024;
pub const MAX_TEXT_OP_BYTES: usize = 256;
/// Every coordinate, size and radius is finite and within ±this.
pub const MAX_COORDINATE: f32 = 32768.0;
pub const MIN_STROKE_WIDTH: f32 = 0.5;
pub const MAX_STROKE_WIDTH: f32 = 16.0;
pub const MAX_CANVASES_PER_SURFACE: usize = 16;

/// A number of the draw list. The model types derive `Eq`, so this compares bit patterns; a
/// checked canvas holds no NaN, and an unchecked one never equals another by accident.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Num(pub f32);

impl PartialEq for Num {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}

impl Eq for Num {}

impl From<f32> for Num {
    fn from(value: f32) -> Self {
        Self(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DataColor {
    Series1,
    Series2,
    Series3,
    Series4,
    Series5,
    Series6,
    Series7,
    Series8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CanvasColor {
    Token(ColorToken),
    Data(DataColor),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Paint {
    pub color: CanvasColor,
    /// Percent, 0 to 100.
    pub opacity: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Stroke {
    pub paint: Paint,
    /// Logical pixels, [`MIN_STROKE_WIDTH`] to [`MAX_STROKE_WIDTH`].
    pub width: Num,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Point {
    pub x: Num,
    pub y: Num,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Rect {
    pub x: Num,
    pub y: Num,
    pub width: Num,
    pub height: Num,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PathSegment {
    MoveTo(Point),
    LineTo(Point),
    /// Control point, end point.
    QuadTo(Point, Point),
    /// Two control points, end point.
    CubicTo(Point, Point, Point),
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CanvasTextSize {
    Small,
    Medium,
    Large,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TextAnchor {
    Start,
    Middle,
    End,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DrawOp {
    Line {
        from: Point,
        to: Point,
        stroke: Stroke,
    },
    Rect {
        bounds: Rect,
        fill: Option<Paint>,
        stroke: Option<Stroke>,
    },
    RoundedRect {
        bounds: Rect,
        /// Clamped by the renderer to half the shorter side.
        radius: Num,
        fill: Option<Paint>,
        stroke: Option<Stroke>,
    },
    Circle {
        center: Point,
        radius: Num,
        fill: Option<Paint>,
        stroke: Option<Stroke>,
    },
    /// At least one of `fill` (non-zero rule) and `stroke`.
    Path {
        segments: Vec<PathSegment>,
        fill: Option<Paint>,
        stroke: Option<Stroke>,
    },
    /// One line; `at` is the baseline at the anchor.
    Text {
        at: Point,
        text: String,
        paint: Paint,
        size: CanvasTextSize,
        anchor: TextAnchor,
        monospace: bool,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CanvasFit {
    /// Drawn at its size; what does not fit the parent is clipped.
    #[default]
    Clip,
    /// Scaled down, keeping the aspect ratio, when the parent is narrower; never up.
    ScaleDown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct CanvasProps {
    /// Logical pixels, 1 to [`MAX_CANVAS_SIDE`].
    pub width: u32,
    pub height: u32,
    /// Required, 1 to [`MAX_DESCRIPTION_BYTES`]: the accessible label and the tooltip, shown in
    /// place of the drawing when it is rejected or the extension stopped.
    pub description: String,
    pub ops: Vec<DrawOp>,
    /// Sends [`CanvasClick`] in canvas coordinates.
    pub clickable: bool,
    pub fit: CanvasFit,
}

/// `ui-event::canvas-click`: a click on a clickable canvas, in canvas coordinates (mapped back
/// through `scale-down` and the panel's scroll by the renderer). A gesture, like `click`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct CanvasClick {
    pub canvas: NodeId,
    pub x: Num,
    pub y: Num,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CanvasError {
    #[error("a canvas needs a description of 1 to {MAX_DESCRIPTION_BYTES} bytes, got {0} bytes")]
    Description(usize),
    #[error("a canvas's {side} is {value}, it must be 1 to {MAX_CANVAS_SIDE}")]
    Size { side: &'static str, value: u32 },
    #[error("a canvas has {0} ops, the cap is {MAX_OPS}")]
    TooManyOps(usize),
    #[error("a canvas has {0} path segments, the cap is {MAX_PATH_SEGMENTS}")]
    TooManySegments(usize),
    #[error("a canvas has {0} bytes of text, the cap is {MAX_TEXT_BYTES}")]
    TooMuchText(usize),
    #[error("op {op}: {problem}")]
    Op { op: usize, problem: String },
    #[error("a canvas can only be in a panel, not in a {0}")]
    NotInPanel(&'static str),
}

impl CanvasError {
    /// The WIT `host-error-code` for this error.
    pub fn code(&self) -> HostErrorCode {
        match self {
            CanvasError::TooManyOps(_)
            | CanvasError::TooManySegments(_)
            | CanvasError::TooMuchText(_) => HostErrorCode::LimitExceeded,
            CanvasError::Description(_)
            | CanvasError::Size { .. }
            | CanvasError::Op { .. }
            | CanvasError::NotInPanel(_) => HostErrorCode::InvalidArgument,
        }
    }
}

/// Bytes of text a canvas carries: its description and every text op. This is what the host
/// counts toward the lifted-text cap of a call (`MAX_LIFTED_TEXT_PER_CALL`).
pub fn canvas_text_len(props: &CanvasProps) -> usize {
    props.description.len()
        + props
            .ops
            .iter()
            .map(|op| match op {
                DrawOp::Text { text, .. } => text.len(),
                _ => 0,
            })
            .sum::<usize>()
}

/// The props rules of design.md §13: sizes, description, caps, finite coordinates within
/// ±[`MAX_COORDINATE`], strokes of [`MIN_STROKE_WIDTH`] to [`MAX_STROKE_WIDTH`], opacity of at
/// most 100, one-line text of at most [`MAX_TEXT_OP_BYTES`], paths that start with `move-to` and
/// draw something, with a fill or a stroke.
pub fn check_canvas_props(props: &CanvasProps) -> Result<(), CanvasError> {
    if !(1..=MAX_DESCRIPTION_BYTES).contains(&props.description.len()) {
        return Err(CanvasError::Description(props.description.len()));
    }
    for (side, value) in [("width", props.width), ("height", props.height)] {
        if !(1..=MAX_CANVAS_SIDE).contains(&value) {
            return Err(CanvasError::Size { side, value });
        }
    }
    if props.ops.len() > MAX_OPS {
        return Err(CanvasError::TooManyOps(props.ops.len()));
    }
    let segments: usize = props
        .ops
        .iter()
        .map(|op| match op {
            DrawOp::Path { segments, .. } => segments.len(),
            _ => 0,
        })
        .sum();
    if segments > MAX_PATH_SEGMENTS {
        return Err(CanvasError::TooManySegments(segments));
    }
    let text = canvas_text_len(props) - props.description.len();
    if text > MAX_TEXT_BYTES {
        return Err(CanvasError::TooMuchText(text));
    }
    for (index, op) in props.ops.iter().enumerate() {
        check_op(op).map_err(|problem| CanvasError::Op { op: index, problem })?;
    }
    Ok(())
}

fn check_op(op: &DrawOp) -> Result<(), String> {
    match op {
        DrawOp::Line { from, to, stroke } => {
            point(from)?;
            point(to)?;
            self::stroke(stroke)
        }
        DrawOp::Rect {
            bounds,
            fill,
            stroke,
        } => {
            rect(bounds)?;
            fill_and_stroke(fill, stroke, false)
        }
        DrawOp::RoundedRect {
            bounds,
            radius,
            fill,
            stroke,
        } => {
            rect(bounds)?;
            length("radius", *radius)?;
            fill_and_stroke(fill, stroke, false)
        }
        DrawOp::Circle {
            center,
            radius,
            fill,
            stroke,
        } => {
            point(center)?;
            length("radius", *radius)?;
            fill_and_stroke(fill, stroke, false)
        }
        DrawOp::Path {
            segments,
            fill,
            stroke,
        } => {
            fill_and_stroke(fill, stroke, true)?;
            let mut started = false;
            let mut draws = false;
            for segment in segments {
                match segment {
                    PathSegment::MoveTo(at) => {
                        point(at)?;
                        started = true;
                    }
                    PathSegment::Close if !started => {
                        return Err("close before a move-to".into());
                    }
                    PathSegment::Close => {}
                    _ if !started => return Err("a path must start with move-to".into()),
                    PathSegment::LineTo(to) => {
                        point(to)?;
                        draws = true;
                    }
                    PathSegment::QuadTo(control, to) => {
                        point(control)?;
                        point(to)?;
                        draws = true;
                    }
                    PathSegment::CubicTo(first, second, to) => {
                        point(first)?;
                        point(second)?;
                        point(to)?;
                        draws = true;
                    }
                }
            }
            if !draws {
                return Err("a path needs a line-to, quad-to or cubic-to".into());
            }
            Ok(())
        }
        DrawOp::Text {
            at, text, paint, ..
        } => {
            point(at)?;
            self::paint(paint)?;
            if text.len() > MAX_TEXT_OP_BYTES {
                return Err(format!(
                    "a text op has {} bytes, the cap is {MAX_TEXT_OP_BYTES}",
                    text.len()
                ));
            }
            if text.contains(['\n', '\r']) {
                return Err("a text op is one line".into());
            }
            Ok(())
        }
    }
}

fn number(what: &str, value: Num) -> Result<(), String> {
    if value.0.is_finite() && value.0.abs() <= MAX_COORDINATE {
        Ok(())
    } else {
        Err(format!(
            "{what} is {}, it must be finite and within ±{MAX_COORDINATE}",
            value.0
        ))
    }
}

fn length(what: &str, value: Num) -> Result<(), String> {
    number(what, value)?;
    if value.0 < 0.0 {
        return Err(format!("{what} is {}, it must not be negative", value.0));
    }
    Ok(())
}

fn point(point: &Point) -> Result<(), String> {
    number("x", point.x)?;
    number("y", point.y)
}

fn rect(rect: &Rect) -> Result<(), String> {
    number("x", rect.x)?;
    number("y", rect.y)?;
    length("width", rect.width)?;
    length("height", rect.height)
}

fn paint(paint: &Paint) -> Result<(), String> {
    if paint.opacity > 100 {
        return Err(format!("opacity is {}, it must be 0 to 100", paint.opacity));
    }
    Ok(())
}

fn stroke(stroke: &Stroke) -> Result<(), String> {
    paint(&stroke.paint)?;
    let width = stroke.width.0;
    if width.is_finite() && (MIN_STROKE_WIDTH..=MAX_STROKE_WIDTH).contains(&width) {
        Ok(())
    } else {
        Err(format!(
            "a stroke is {width} wide, it must be {MIN_STROKE_WIDTH} to {MAX_STROKE_WIDTH}"
        ))
    }
}

fn fill_and_stroke(
    fill: &Option<Paint>,
    stroke: &Option<Stroke>,
    need_one: bool,
) -> Result<(), String> {
    if need_one && fill.is_none() && stroke.is_none() {
        return Err("a path needs a fill or a stroke".into());
    }
    if let Some(fill) = fill {
        paint(fill)?;
    }
    if let Some(stroke) = stroke {
        self::stroke(stroke)?;
    }
    Ok(())
}

/// Canvases go only in panel trees: status items, modals, pickers and decorations sit next to
/// host UI and would be easy to spoof (design.md §13, threats). List rows are refused by the
/// tree ([`crate::v0_2::tree::Tree::from_row`]).
pub fn check_canvas_placement(kind: Option<&SurfaceKind>) -> Result<(), CanvasError> {
    match kind {
        Some(SurfaceKind::Panel(_)) => Ok(()),
        Some(SurfaceKind::StatusItem(_)) => Err(CanvasError::NotInPanel("status item")),
        Some(SurfaceKind::Modal(_)) => Err(CanvasError::NotInPanel("modal")),
        None => Err(CanvasError::NotInPanel("decoration")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(x: f32, y: f32) -> Point {
        Point {
            x: Num(x),
            y: Num(y),
        }
    }

    fn paint() -> Paint {
        Paint {
            color: CanvasColor::Data(DataColor::Series1),
            opacity: 100,
        }
    }

    fn stroke(width: f32) -> Stroke {
        Stroke {
            paint: paint(),
            width: Num(width),
        }
    }

    fn line(x: f32) -> DrawOp {
        DrawOp::Line {
            from: pt(0.0, 0.0),
            to: pt(x, 1.0),
            stroke: stroke(1.0),
        }
    }

    fn text(text: &str) -> DrawOp {
        DrawOp::Text {
            at: pt(1.0, 1.0),
            text: text.into(),
            paint: paint(),
            size: CanvasTextSize::Small,
            anchor: TextAnchor::Start,
            monospace: false,
        }
    }

    fn path(segments: Vec<PathSegment>) -> DrawOp {
        DrawOp::Path {
            segments,
            fill: Some(paint()),
            stroke: None,
        }
    }

    fn canvas(ops: Vec<DrawOp>) -> CanvasProps {
        CanvasProps {
            width: 240,
            height: 120,
            description: "Commits per day".into(),
            ops,
            clickable: false,
            fit: CanvasFit::Clip,
        }
    }

    fn check(ops: Vec<DrawOp>) -> Result<(), CanvasError> {
        check_canvas_props(&canvas(ops))
    }

    #[test]
    fn caps_at_and_past_their_limit() {
        assert_eq!(check(vec![line(1.0); MAX_OPS]), Ok(()));
        assert_eq!(
            check(vec![line(1.0); MAX_OPS + 1]),
            Err(CanvasError::TooManyOps(MAX_OPS + 1))
        );
        let segments = |count: usize| {
            let mut segments = vec![PathSegment::MoveTo(pt(0.0, 0.0))];
            segments.extend(vec![PathSegment::LineTo(pt(1.0, 1.0)); count - 1]);
            path(segments)
        };
        assert_eq!(check(vec![segments(MAX_PATH_SEGMENTS)]), Ok(()));
        assert_eq!(
            check(vec![
                segments(MAX_PATH_SEGMENTS / 2),
                segments(MAX_PATH_SEGMENTS / 2 + 1)
            ]),
            Err(CanvasError::TooManySegments(MAX_PATH_SEGMENTS + 1))
        );
        let texts = |total: usize| {
            let mut ops = vec![text(&"a".repeat(MAX_TEXT_OP_BYTES)); total / MAX_TEXT_OP_BYTES];
            ops.push(text(&"a".repeat(total % MAX_TEXT_OP_BYTES)));
            ops
        };
        assert_eq!(check(texts(MAX_TEXT_BYTES)), Ok(()));
        assert_eq!(
            check(texts(MAX_TEXT_BYTES + 1)),
            Err(CanvasError::TooMuchText(MAX_TEXT_BYTES + 1))
        );
        assert_eq!(
            CanvasError::TooManyOps(0).code(),
            HostErrorCode::LimitExceeded
        );
    }

    #[test]
    fn numbers_must_be_finite_and_in_range() {
        for bad in [
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            32769.0,
            -32769.0,
        ] {
            assert!(check(vec![line(bad)]).is_err(), "{bad}");
        }
        assert_eq!(check(vec![line(32768.0), line(-32768.0)]), Ok(()));
        for width in [0.4, 16.1, f32::NAN] {
            let op = DrawOp::Line {
                from: pt(0.0, 0.0),
                to: pt(1.0, 1.0),
                stroke: stroke(width),
            };
            assert!(check(vec![op]).is_err(), "stroke {width}");
        }
        let negative = DrawOp::Circle {
            center: pt(1.0, 1.0),
            radius: Num(-1.0),
            fill: Some(paint()),
            stroke: None,
        };
        assert!(check(vec![negative]).is_err());
        let opaque = DrawOp::Rect {
            bounds: Rect::default(),
            fill: Some(Paint {
                opacity: 101,
                ..paint()
            }),
            stroke: None,
        };
        assert!(check(vec![opaque]).is_err());
    }

    #[test]
    fn description_size_text_and_paths() {
        let mut no_description = canvas(Vec::new());
        no_description.description.clear();
        assert_eq!(
            check_canvas_props(&no_description),
            Err(CanvasError::Description(0))
        );
        no_description.description = "a".repeat(MAX_DESCRIPTION_BYTES + 1);
        assert!(check_canvas_props(&no_description).is_err());
        for (width, height) in [(0, 1), (1, 0), (MAX_CANVAS_SIDE + 1, 1)] {
            let mut sized = canvas(Vec::new());
            sized.width = width;
            sized.height = height;
            assert!(check_canvas_props(&sized).is_err(), "{width} x {height}");
        }
        assert!(check(vec![text("two\nlines")]).is_err());
        assert!(check(vec![text(&"a".repeat(MAX_TEXT_OP_BYTES + 1))]).is_err());
        assert!(
            check(vec![path(vec![PathSegment::MoveTo(pt(0.0, 0.0))])]).is_err(),
            "only move-to"
        );
        assert!(
            check(vec![path(vec![PathSegment::Close])]).is_err(),
            "close first"
        );
        assert!(check(vec![path(vec![PathSegment::LineTo(pt(1.0, 1.0))])]).is_err());
        let unpainted = DrawOp::Path {
            segments: vec![
                PathSegment::MoveTo(pt(0.0, 0.0)),
                PathSegment::LineTo(pt(1.0, 1.0)),
            ],
            fill: None,
            stroke: None,
        };
        assert!(check(vec![unpainted]).is_err());
        assert_eq!(
            check(vec![path(vec![
                PathSegment::MoveTo(pt(0.0, 0.0)),
                PathSegment::QuadTo(pt(1.0, 0.0), pt(1.0, 1.0)),
                PathSegment::CubicTo(pt(2.0, 0.0), pt(2.0, 2.0), pt(3.0, 3.0)),
                PathSegment::Close,
            ])]),
            Ok(())
        );
    }

    #[test]
    fn text_counts_for_the_lifted_text_cap() {
        assert_eq!(
            canvas_text_len(&canvas(vec![text("abc"), line(1.0), text("de")])),
            "Commits per day".len() + 5
        );
    }

    #[test]
    fn nan_compares_by_bits() {
        assert_eq!(Num(f32::NAN), Num(f32::NAN));
        assert_ne!(Num(0.0), Num(-0.0));
    }
}
