//! The todo-21 shape-tool scene objects: freehand pencil paths, straight
//! lines, translucent chisel-cap marker strokes, and non-destructive color
//! inversion regions.
//!
//! All four are plain data + paint dispatch through [`PaintSink`] (the core
//! crate's renderer-agnostic contract); geometry lives in scene coordinates
//! (global logical px, f32). The marker's chisel cap is the Qt `SquareCap`
//! parity shape: a flat-ended oriented quad extending half the width beyond
//! each endpoint (Flameshot's `QPen` default cap), which keeps the
//! highlighter look without a renderer line-cap vocabulary.

use serde::{Deserialize, Serialize};

use super::{Color, PaintSink, Point, Rect, ToolObject, ToolObjectData};

/// A freehand pencil stroke: a polyline with round caps/joins (the
/// renderer's annotation default) at `thickness` width.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PencilPath {
    /// The stroke points in draw order (simplified at commit - the tool's
    /// Ramer-Douglas-Peucker pass, plan todo 21).
    pub points: Vec<Point>,
    /// Stroke color.
    pub color: Color,
    /// Stroke thickness.
    pub thickness: f32,
}

impl PencilPath {
    /// Creates a pencil stroke.
    #[must_use]
    pub fn new(points: Vec<Point>, color: Color, thickness: f32) -> Self {
        Self {
            points,
            color,
            thickness,
        }
    }
}

impl ToolObject for PencilPath {
    fn type_id(&self) -> &'static str {
        "pencil"
    }

    fn bounding_rect(&self) -> Rect {
        bounds_of(&self.points, self.thickness)
    }

    fn paint(&self, sink: &mut dyn PaintSink) {
        if self.points.len() >= 2 {
            sink.stroke_polyline(&self.points, self.color, self.thickness);
        }
    }

    fn to_data(&self) -> ToolObjectData {
        ToolObjectData::Pencil(self.clone())
    }
}

/// A straight two-point line stroke.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LineObject {
    /// Start point.
    pub from: Point,
    /// End point.
    pub to: Point,
    /// Line color.
    pub color: Color,
    /// Line thickness.
    pub thickness: f32,
}

impl LineObject {
    /// Creates a line stroke.
    #[must_use]
    pub fn new(from: Point, to: Point, color: Color, thickness: f32) -> Self {
        Self {
            from,
            to,
            color,
            thickness,
        }
    }
}

impl ToolObject for LineObject {
    fn type_id(&self) -> &'static str {
        "line"
    }

    fn bounding_rect(&self) -> Rect {
        bounds_of(&[self.from, self.to], self.thickness)
    }

    fn paint(&self, sink: &mut dyn PaintSink) {
        sink.draw_line(self.from, self.to, self.color, self.thickness);
    }

    fn to_data(&self) -> ToolObjectData {
        ToolObjectData::Line(self.clone())
    }
}

/// A translucent highlighter stroke between two points, painted as a
/// chisel-cap (flat-ended) quad of `width` - the marker color carries the
/// ~0.5 blend alpha (plan todo 21).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarkerObject {
    /// Start point.
    pub from: Point,
    /// End point.
    pub to: Point,
    /// Marker color INCLUDING the translucent alpha.
    pub color: Color,
    /// Stroke width (`[tools.marker].size`).
    pub width: f32,
}

impl MarkerObject {
    /// Creates a marker stroke.
    #[must_use]
    pub fn new(from: Point, to: Point, color: Color, width: f32) -> Self {
        Self {
            from,
            to,
            color,
            width,
        }
    }

    /// The chisel-cap quad corners (flat ends perpendicular to the stroke,
    /// extended half the width beyond each endpoint - Qt `SquareCap` parity);
    /// a zero-length stroke yields the centered square dot. `None` when the
    /// width paints no ink.
    #[must_use]
    pub fn chisel_polygon(&self) -> Option<Vec<Point>> {
        if !self.width.is_finite() || self.width <= 0.0 {
            return None;
        }
        let half = self.width * 0.5;
        let (dx, dy) = (self.to.x - self.from.x, self.to.y - self.from.y);
        let len = (dx * dx + dy * dy).sqrt();
        let (dir_x, dir_y) = if len.is_finite() && len > 0.0 {
            (dx / len, dy / len)
        } else {
            (1.0, 0.0)
        };
        let start = Point::new(self.from.x - dir_x * half, self.from.y - dir_y * half);
        let end = Point::new(self.to.x + dir_x * half, self.to.y + dir_y * half);
        let (nx, ny) = (-dir_y * half, dir_x * half);
        Some(vec![
            Point::new(start.x + nx, start.y + ny),
            Point::new(end.x + nx, end.y + ny),
            Point::new(end.x - nx, end.y - ny),
            Point::new(start.x - nx, start.y - ny),
        ])
    }
}

impl ToolObject for MarkerObject {
    fn type_id(&self) -> &'static str {
        "marker"
    }

    fn bounding_rect(&self) -> Rect {
        match self.chisel_polygon() {
            Some(polygon) => bounds_of(&polygon, 0.0),
            None => Rect::from_points(self.from, self.to),
        }
    }

    fn paint(&self, sink: &mut dyn PaintSink) {
        if let Some(polygon) = self.chisel_polygon() {
            sink.fill_polygon(&polygon, self.color);
        }
    }

    fn to_data(&self) -> ToolObjectData {
        ToolObjectData::Marker(self.clone())
    }
}

/// A non-destructive color-inversion region: painting emits
/// [`PaintSink::invert_region`], so the frame pixels themselves are never
/// modified (undo = remove the object; the plan todo-21 filter-object
/// contract).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InvertObject {
    /// The inverted region.
    pub rect: Rect,
}

impl InvertObject {
    /// Creates an inversion region.
    #[must_use]
    pub const fn new(rect: Rect) -> Self {
        Self { rect }
    }
}

impl ToolObject for InvertObject {
    fn type_id(&self) -> &'static str {
        "invert"
    }

    fn bounding_rect(&self) -> Rect {
        self.rect
    }

    fn paint(&self, sink: &mut dyn PaintSink) {
        sink.invert_region(self.rect);
    }

    fn to_data(&self) -> ToolObjectData {
        ToolObjectData::Invert(self.clone())
    }
}

/// The exact ink bounds of `points` grown by the stroke half-width; a
/// degenerate point set yields a zero-size rect at the origin.
fn bounds_of(points: &[Point], stroke: f32) -> Rect {
    let mut bounds = points.iter().fold(None, |acc: Option<Rect>, point| {
        Some(match acc {
            None => Rect::new(point.x, point.y, 0.0, 0.0),
            Some(rect) => {
                let min_x = rect.x.min(point.x);
                let min_y = rect.y.min(point.y);
                let max_x = (rect.x + rect.width).max(point.x);
                let max_y = (rect.y + rect.height).max(point.y);
                Rect::new(min_x, min_y, max_x - min_x, max_y - min_y)
            }
        })
    });
    let grow = if stroke.is_finite() {
        stroke.max(0.0) * 0.5
    } else {
        0.0
    };
    if let Some(rect) = &mut bounds {
        rect.x -= grow;
        rect.y -= grow;
        rect.width += grow * 2.0;
        rect.height += grow * 2.0;
    }
    bounds.unwrap_or_else(|| Rect::new(0.0, 0.0, 0.0, 0.0))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

    use super::*;
    use crate::scene::test_support::{Call, RecSink};

    const RED: Color = Color::new(255, 0, 0, 255);
    const TRANSLUCENT: Color = Color::new(255, 0, 0, 128);

    fn point(x: f32, y: f32) -> Point {
        Point::new(x, y)
    }

    #[test]
    fn pencil_paints_one_polyline_with_exact_ink_bounds() {
        let path = PencilPath::new(
            vec![point(10.0, 20.0), point(30.0, 5.0), point(50.0, 40.0)],
            RED,
            4.0,
        );
        let mut sink = RecSink::default();
        path.paint(&mut sink);
        assert_eq!(
            sink.calls,
            vec![Call::Polyline(path.points.clone(), RED, 4.0)]
        );
        // Points span x[10,50] y[5,40], grown by thickness/2 = 2.
        assert_eq!(path.bounding_rect(), Rect::new(8.0, 3.0, 44.0, 39.0));
        assert_eq!(path.type_id(), "pencil");
    }

    #[test]
    fn pencil_with_a_single_point_paints_nothing() {
        let path = PencilPath::new(vec![point(1.0, 1.0)], RED, 2.0);
        let mut sink = RecSink::default();
        path.paint(&mut sink);
        assert!(sink.calls.is_empty());
        assert_eq!(path.bounding_rect(), Rect::new(0.0, 0.0, 2.0, 2.0));
    }

    #[test]
    fn line_bounds_grow_by_the_stroke_half_width() {
        let line = LineObject::new(point(0.0, 0.0), point(10.0, 5.0), RED, 2.0);
        assert_eq!(line.bounding_rect(), Rect::new(-1.0, -1.0, 12.0, 7.0));
        let mut sink = RecSink::default();
        line.paint(&mut sink);
        assert_eq!(
            sink.calls,
            vec![Call::Line(point(0.0, 0.0), point(10.0, 5.0), RED, 2.0)]
        );
        assert_eq!(line.type_id(), "line");
    }

    #[test]
    fn marker_chisel_is_a_square_cap_quad() {
        let marker = MarkerObject::new(point(10.0, 10.0), point(50.0, 10.0), TRANSLUCENT, 8.0);
        let polygon = marker.chisel_polygon().expect("positive width");
        // Extends 4px beyond each endpoint, 4px to each side.
        assert_eq!(
            polygon,
            vec![
                point(6.0, 14.0),
                point(54.0, 14.0),
                point(54.0, 6.0),
                point(6.0, 6.0),
            ]
        );
        let mut sink = RecSink::default();
        marker.paint(&mut sink);
        assert_eq!(sink.calls, vec![Call::Polygon(polygon, TRANSLUCENT)]);
        assert_eq!(marker.bounding_rect(), Rect::new(6.0, 6.0, 48.0, 8.0));
        assert_eq!(marker.type_id(), "marker");
    }

    #[test]
    fn marker_degenerates_to_a_dot_and_rejects_empty_width() {
        let dot = MarkerObject::new(point(20.0, 20.0), point(20.0, 20.0), TRANSLUCENT, 10.0);
        let polygon = dot.chisel_polygon().expect("positive width");
        assert_eq!(
            polygon,
            vec![
                point(15.0, 25.0),
                point(25.0, 25.0),
                point(25.0, 15.0),
                point(15.0, 15.0),
            ]
        );
        for width in [0.0, -3.0, f32::NAN] {
            let marker = MarkerObject::new(point(0.0, 0.0), point(9.0, 0.0), TRANSLUCENT, width);
            assert!(marker.chisel_polygon().is_none());
            let mut sink = RecSink::default();
            marker.paint(&mut sink);
            assert!(sink.calls.is_empty(), "width {width} paints nothing");
        }
    }

    #[test]
    fn invert_emits_the_region_filter_and_keeps_the_exact_rect() {
        let rect = Rect::new(100.0, 50.0, 200.0, 80.0);
        let invert = InvertObject::new(rect);
        let mut sink = RecSink::default();
        invert.paint(&mut sink);
        assert_eq!(sink.calls, vec![Call::Invert(rect)]);
        assert_eq!(invert.bounding_rect(), rect);
        assert_eq!(invert.type_id(), "invert");
    }

    #[test]
    fn data_roundtrips_every_new_object() {
        let objects: Vec<Box<dyn ToolObject>> = vec![
            Box::new(PencilPath::new(
                vec![point(0.0, 0.0), point(5.0, 5.0)],
                RED,
                3.0,
            )),
            Box::new(LineObject::new(point(0.0, 0.0), point(5.0, 5.0), RED, 3.0)),
            Box::new(MarkerObject::new(
                point(0.0, 0.0),
                point(5.0, 5.0),
                TRANSLUCENT,
                12.0,
            )),
            Box::new(InvertObject::new(Rect::new(1.0, 2.0, 3.0, 4.0))),
        ];
        for object in &objects {
            let data = object.to_data();
            let restored = data.clone().into_object();
            assert_eq!(restored.type_id(), object.type_id());
            assert_eq!(restored.to_data(), data);
            assert_eq!(restored.bounding_rect(), object.bounding_rect());
        }
        // The serde tags are the stable PascalCase variant names.
        let text = toml::to_string(&objects[0].to_data()).unwrap_or_default();
        assert!(text.contains("Pencil"), "{text}");
    }
}
