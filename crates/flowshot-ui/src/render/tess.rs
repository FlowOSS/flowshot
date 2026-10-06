//! Lyon tessellation bridge: display-list shapes to flat-shaded triangles.
//!
//! Geometry is tessellated in physical-pixel space exactly as the display
//! list specifies it (stroke widths are physical px).
//! Antialiasing comes from the 4x-multisampled render target the renderer
//! draws into (lyon emits exact geometry; the resolve averages edge
//! coverage), which is why fills and strokes share one flat-color pipeline.
//! Tessellation failures are logged and skip the shape - a degraded frame,
//! never a panic (the crate-wide no-panic rule).

use lyon::math::{Box2D, Point as LPoint, point};
use lyon::path::{Builder as PathBuilderOwner, Path, Winding};
use lyon::tessellation::{
    BuffersBuilder, FillOptions, FillTessellator, FillVertex, FillVertexConstructor, StrokeOptions,
    StrokeTessellator, StrokeVertex, StrokeVertexConstructor, VertexBuffers,
};

use super::geom::{Point, Rect};
use super::list::Shape;

/// Path builder flavor exposing the shape helpers (`add_rectangle`,
/// `add_ellipse`) as inherent methods.
type SvgBuilder = PathBuilderOwner;

/// Interleaved flat vertex: `[x, y, r, g, b, a]`, color premultiplied
/// linear. A plain `f32` array so staging uploads use bytemuck's built-in
/// `Pod` impls (no derive under `forbid(unsafe_code)`).
pub(crate) type FlatVertex = [f32; 6];

/// Curve flattening tolerance in px. Finer than lyon's 0.1 default: chord
/// error stays well under the 8x MSAA sample spacing, so curve edges are
/// limited by sample coverage, not flattening (visually and for the
/// cross-rasterizer parity harness).
const TOLERANCE: f32 = 0.05;

/// Accumulated triangle mesh: flat vertices plus `u32` indices.
pub(crate) type MeshBuffers = VertexBuffers<FlatVertex, u32>;

/// Reusable lyon tessellators (they carry scratch allocations).
pub(crate) struct Tessellator {
    fill: FillTessellator,
    stroke: StrokeTessellator,
}

impl std::fmt::Debug for Tessellator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad("Tessellator { .. }")
    }
}

impl Default for Tessellator {
    fn default() -> Self {
        Self::new()
    }
}

impl Tessellator {
    pub(crate) fn new() -> Self {
        Self {
            fill: FillTessellator::new(),
            stroke: StrokeTessellator::new(),
        }
    }

    /// Fills `shape` with a premultiplied-linear `color` (even-odd rule).
    pub(crate) fn fill_shape(&mut self, shape: &Shape, color: [f32; 4], out: &mut MeshBuffers) {
        let options = FillOptions::even_odd().with_tolerance(TOLERANCE);
        match shape {
            Shape::Rect { rect, radius: 0.0 } if rect.is_valid() => {
                let result = self.fill.tessellate_rectangle(
                    &box2d(*rect),
                    &options,
                    &mut BuffersBuilder::new(out, Ctor { color }),
                );
                log_tessellation_failure(result, "rect fill");
            }
            shape => {
                let Some(path) = shape_path(shape) else {
                    return;
                };
                let result = self.fill.tessellate_path(
                    &path,
                    &options,
                    &mut BuffersBuilder::new(out, Ctor { color }),
                );
                log_tessellation_failure(result, "fill");
            }
        }
    }

    /// Strokes `shape` with `width` in physical px; round caps and joins are
    /// the renderer's annotation-friendly default (flameshot-parity look,
    /// the shape tools draw through this path).
    pub(crate) fn stroke_shape(
        &mut self,
        shape: &Shape,
        width: f32,
        color: [f32; 4],
        out: &mut MeshBuffers,
    ) {
        if !width.is_finite() || width <= 0.0 {
            tracing::debug!(width, "non-positive stroke width skipped");
            return;
        }
        let Some(path) = shape_path(shape) else {
            return;
        };
        let options = StrokeOptions::default()
            .with_tolerance(TOLERANCE)
            .with_line_width(width)
            .with_line_cap(lyon::path::LineCap::Round)
            .with_line_join(lyon::path::LineJoin::Round);
        let result = self.stroke.tessellate_path(
            &path,
            &options,
            &mut BuffersBuilder::new(out, Ctor { color }),
        );
        log_tessellation_failure(result, "stroke");
    }

    /// The dim layer: `bounds` minus every cutout, even-odd filled in one
    /// mesh (the selection cutout).
    pub(crate) fn dim(
        &mut self,
        bounds: Rect,
        cutouts: &[Rect],
        color: [f32; 4],
        out: &mut MeshBuffers,
    ) {
        if !bounds.is_valid() {
            tracing::debug!("invalid dim bounds skipped");
            return;
        }
        let mut builder = Path::builder();
        add_rect(&mut builder, bounds);
        for cutout in cutouts {
            if cutout.is_valid() {
                add_rect(&mut builder, *cutout);
            }
        }
        let path = builder.build();
        let result = self.fill.tessellate_path(
            &path,
            &FillOptions::even_odd().with_tolerance(TOLERANCE),
            &mut BuffersBuilder::new(out, Ctor { color }),
        );
        log_tessellation_failure(result, "dim");
    }
}

/// Converts a display-list shape to a lyon path; `None` when the shape is
/// degenerate or fill-only-invalid (a line has no fillable interior).
fn shape_path(shape: &Shape) -> Option<Path> {
    let mut builder = Path::builder();
    match shape {
        Shape::Rect { rect, radius } => {
            if !rect.is_valid() || !radius.is_finite() {
                return None;
            }
            add_rounded_rect(&mut builder, *rect, radius.max(0.0));
        }
        Shape::Ellipse { center, radii } => {
            if !center.x.is_finite() || !center.y.is_finite() || !radii.is_valid() {
                return None;
            }
            if radii.width <= 0.0 || radii.height <= 0.0 {
                return None;
            }
            add_ellipse(&mut builder, *center, radii.width, radii.height);
        }
        Shape::Line { from, to } => {
            if !from.x.is_finite() || !to.x.is_finite() || !from.y.is_finite() || !to.y.is_finite()
            {
                return None;
            }
            builder.begin(lpoint(*from));
            builder.line_to(lpoint(*to));
            builder.end(false);
        }
        Shape::Polyline { points, closed } => {
            let mut iter = points.iter();
            let first = iter.next()?;
            builder.begin(lpoint(*first));
            for next in iter {
                builder.line_to(lpoint(*next));
            }
            builder.end(*closed);
        }
        Shape::Path { segments } => {
            use super::list::PathSegment;
            // lyon's builder panics on out-of-order verbs; the guards keep a
            // malformed display list a skipped shape instead (no panics).
            let mut open = false;
            for segment in segments {
                match *segment {
                    PathSegment::MoveTo(at) => {
                        builder.begin(lpoint(at));
                        open = true;
                    }
                    PathSegment::LineTo(at) => {
                        if open {
                            builder.line_to(lpoint(at));
                        }
                    }
                    PathSegment::QuadTo(ctrl, to) => {
                        if open {
                            builder.quadratic_bezier_to(lpoint(ctrl), lpoint(to));
                        }
                    }
                    PathSegment::CubeTo(ctrl1, ctrl2, to) => {
                        if open {
                            builder.cubic_bezier_to(lpoint(ctrl1), lpoint(ctrl2), lpoint(to));
                        }
                    }
                    PathSegment::Close => {
                        if open {
                            builder.close();
                            open = false;
                        }
                    }
                }
            }
            if open {
                builder.end(false);
            }
        }
    }
    Some(builder.build())
}

fn add_rect(builder: &mut SvgBuilder, rect: Rect) {
    builder.add_rectangle(&box2d(rect), Winding::Positive);
}

/// Rounded rect via quadratic corner arcs; the radius is clamped to half the
/// smaller side so the corners never cross.
fn add_rounded_rect(builder: &mut SvgBuilder, rect: Rect, radius: f32) {
    let r = radius
        .min(rect.size.width * 0.5)
        .min(rect.size.height * 0.5);
    if r <= 0.0 {
        add_rect(builder, rect);
        return;
    }
    let (x0, y0) = (rect.origin.x, rect.origin.y);
    let (x1, y1) = (rect.right(), rect.bottom());
    builder.begin(point(x0 + r, y0));
    builder.line_to(point(x1 - r, y0));
    builder.quadratic_bezier_to(point(x1, y0), point(x1, y0 + r));
    builder.line_to(point(x1, y1 - r));
    builder.quadratic_bezier_to(point(x1, y1), point(x1 - r, y1));
    builder.line_to(point(x0 + r, y1));
    builder.quadratic_bezier_to(point(x0, y1), point(x0, y1 - r));
    builder.line_to(point(x0, y0 + r));
    builder.quadratic_bezier_to(point(x0, y0), point(x0 + r, y0));
    builder.end(true);
}

/// Ellipse as four kappa cubic segments (radial error ~0.027% - under a
/// tenth of the flattening tolerance at UI scales). Deliberately NOT lyon's
/// `add_ellipse`: its eight quadratic arcs overshoot the true ellipse by
/// ~0.3% of the radius (0.3 px at r=100, visible faceting on large shapes
/// and a cross-rasterizer parity outlier).
fn add_ellipse(builder: &mut SvgBuilder, center: Point, rx: f32, ry: f32) {
    const KAPPA: f32 = 0.552_284_7;
    let (cx, cy) = (center.x, center.y);
    let (ox, oy) = (rx * KAPPA, ry * KAPPA);
    builder.begin(point(cx - rx, cy));
    builder.cubic_bezier_to(
        point(cx - rx, cy - oy),
        point(cx - ox, cy - ry),
        point(cx, cy - ry),
    );
    builder.cubic_bezier_to(
        point(cx + ox, cy - ry),
        point(cx + rx, cy - oy),
        point(cx + rx, cy),
    );
    builder.cubic_bezier_to(
        point(cx + rx, cy + oy),
        point(cx + ox, cy + ry),
        point(cx, cy + ry),
    );
    builder.cubic_bezier_to(
        point(cx - ox, cy + ry),
        point(cx - rx, cy + oy),
        point(cx - rx, cy),
    );
    builder.end(true);
}

fn box2d(rect: Rect) -> Box2D {
    Box2D::new(
        point(rect.origin.x, rect.origin.y),
        point(rect.right(), rect.bottom()),
    )
}

const fn lpoint(p: Point) -> LPoint {
    LPoint::new(p.x, p.y)
}

fn log_tessellation_failure(result: Result<(), lyon::tessellation::TessellationError>, what: &str) {
    if let Err(error) = result {
        tracing::error!(error = %error, shape = what, "tessellation failed; shape skipped");
    }
}

/// Attaches the current draw color to every tessellated vertex.
struct Ctor {
    color: [f32; 4],
}

impl Ctor {
    fn vertex(&self, position: LPoint) -> FlatVertex {
        [
            position.x,
            position.y,
            self.color[0],
            self.color[1],
            self.color[2],
            self.color[3],
        ]
    }
}

impl FillVertexConstructor<FlatVertex> for Ctor {
    fn new_vertex(&mut self, vertex: FillVertex) -> FlatVertex {
        self.vertex(vertex.position())
    }
}

impl StrokeVertexConstructor<FlatVertex> for Ctor {
    fn new_vertex(&mut self, vertex: StrokeVertex) -> FlatVertex {
        self.vertex(vertex.position())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::*;
    use crate::render::color::Color;
    use crate::render::list::PathSegment;

    const OPAQUE: [f32; 4] = [1.0, 0.5, 0.25, 1.0];

    fn bounds_of(buffers: &MeshBuffers) -> (f32, f32, f32, f32) {
        let mut min = (f32::MAX, f32::MAX);
        let mut max = (f32::MIN, f32::MIN);
        for vertex in &buffers.vertices {
            min.0 = min.0.min(vertex[0]);
            min.1 = min.1.min(vertex[1]);
            max.0 = max.0.max(vertex[0]);
            max.1 = max.1.max(vertex[1]);
        }
        (min.0, min.1, max.0, max.1)
    }

    #[test]
    fn sharp_rect_fill_uses_the_fast_path() {
        let mut tess = Tessellator::new();
        let mut out = MeshBuffers::new();
        tess.fill_shape(
            &Shape::Rect {
                rect: Rect::from_parts(10.0, 20.0, 30.0, 40.0),
                radius: 0.0,
            },
            OPAQUE,
            &mut out,
        );
        assert_eq!(out.vertices.len(), 4);
        assert_eq!(out.indices.len(), 6);
        assert_eq!(bounds_of(&out), (10.0, 20.0, 40.0, 60.0));
        assert!(out.vertices.iter().all(|v| v[2..] == OPAQUE));
    }

    #[test]
    fn rounded_rect_adds_corner_geometry() {
        let mut tess = Tessellator::new();
        let mut sharp = MeshBuffers::new();
        let mut rounded = MeshBuffers::new();
        let rect = Rect::from_parts(0.0, 0.0, 100.0, 50.0);
        tess.fill_shape(&Shape::Rect { rect, radius: 0.0 }, OPAQUE, &mut sharp);
        tess.fill_shape(&Shape::Rect { rect, radius: 8.0 }, OPAQUE, &mut rounded);
        assert!(rounded.vertices.len() > sharp.vertices.len());
        assert_eq!(bounds_of(&rounded), (0.0, 0.0, 100.0, 50.0));
    }

    #[test]
    fn radius_clamps_to_half_the_smaller_side() {
        let mut tess = Tessellator::new();
        let mut out = MeshBuffers::new();
        tess.fill_shape(
            &Shape::Rect {
                rect: Rect::from_parts(0.0, 0.0, 40.0, 10.0),
                radius: 500.0,
            },
            OPAQUE,
            &mut out,
        );
        assert_eq!(bounds_of(&out), (0.0, 0.0, 40.0, 10.0));
    }

    #[test]
    fn stroke_width_is_physical_and_centered_on_the_path() {
        let mut tess = Tessellator::new();
        let mut out = MeshBuffers::new();
        tess.stroke_shape(
            &Shape::Line {
                from: Point::new(0.0, 0.0),
                to: Point::new(10.0, 0.0),
            },
            2.0,
            OPAQUE,
            &mut out,
        );
        assert!(out.vertices.len() >= 4);
        let (_, min_y, _, max_y) = bounds_of(&out);
        assert!(min_y <= -0.9 && max_y >= 0.9, "stroke spans +/- width/2");
    }

    #[test]
    fn dim_cutout_produces_a_hole_mesh() {
        let mut tess = Tessellator::new();
        let mut plain = MeshBuffers::new();
        let mut dimmed = MeshBuffers::new();
        let bounds = Rect::from_parts(0.0, 0.0, 200.0, 100.0);
        let cutout = Rect::from_parts(50.0, 25.0, 100.0, 50.0);
        tess.fill_shape(
            &Shape::Rect {
                rect: bounds,
                radius: 0.0,
            },
            OPAQUE,
            &mut plain,
        );
        tess.dim(bounds, &[cutout], OPAQUE, &mut dimmed);
        assert!(dimmed.indices.len() > plain.indices.len());
        assert_eq!(bounds_of(&dimmed), (0.0, 0.0, 200.0, 100.0));
    }

    #[test]
    fn ellipse_and_path_tessellate() {
        let mut tess = Tessellator::new();
        let mut ellipse = MeshBuffers::new();
        tess.fill_shape(
            &Shape::Ellipse {
                center: Point::new(50.0, 50.0),
                radii: lyon_size(30.0, 20.0),
            },
            OPAQUE,
            &mut ellipse,
        );
        assert!(ellipse.vertices.len() > 8);
        let mut path = MeshBuffers::new();
        tess.fill_shape(
            &Shape::Path {
                segments: vec![
                    PathSegment::MoveTo(Point::new(0.0, 0.0)),
                    PathSegment::LineTo(Point::new(10.0, 0.0)),
                    PathSegment::CubeTo(
                        Point::new(10.0, 5.0),
                        Point::new(5.0, 10.0),
                        Point::new(0.0, 10.0),
                    ),
                    PathSegment::Close,
                ],
            },
            OPAQUE,
            &mut path,
        );
        assert_ne!(path.vertices, [] as [[f32; 6]; 0]);
    }

    fn lyon_size(width: f32, height: f32) -> crate::render::geom::Size {
        crate::render::geom::Size::new(width, height)
    }

    #[test]
    fn degenerate_input_is_skipped_without_geometry() {
        let mut tess = Tessellator::new();
        let mut out = MeshBuffers::new();
        tess.fill_shape(
            &Shape::Polyline {
                points: Vec::new(),
                closed: false,
            },
            OPAQUE,
            &mut out,
        );
        tess.fill_shape(
            &Shape::Rect {
                rect: Rect::from_parts(f32::NAN, 0.0, 10.0, 10.0),
                radius: 0.0,
            },
            OPAQUE,
            &mut out,
        );
        tess.stroke_shape(
            &Shape::Line {
                from: Point::new(0.0, 0.0),
                to: Point::new(1.0, 1.0),
            },
            0.0,
            OPAQUE,
            &mut out,
        );
        tess.stroke_shape(
            &Shape::Line {
                from: Point::new(0.0, 0.0),
                to: Point::new(1.0, 1.0),
            },
            f32::NAN,
            OPAQUE,
            &mut out,
        );
        assert!(out.vertices.is_empty() && out.indices.is_empty());
    }

    #[test]
    fn colors_come_from_tokens_not_literals() {
        let accent = Color::from_hex_token("#6366F1")
            .unwrap_or_else(|| panic!("token"))
            .premultiplied_linear();
        let mut tess = Tessellator::new();
        let mut out = MeshBuffers::new();
        tess.fill_shape(
            &Shape::Rect {
                rect: Rect::from_parts(0.0, 0.0, 4.0, 4.0),
                radius: 0.0,
            },
            accent,
            &mut out,
        );
        assert!(out.vertices.iter().all(|v| v[2..] == accent));
    }
}
