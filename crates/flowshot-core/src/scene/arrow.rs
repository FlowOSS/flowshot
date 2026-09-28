//! The arrow annotation object (draft F27 arrow math).
//!
//! Clean-room reimplementation of the Flameshot `ArrowTool` geometry
//! (`arrowtool.cpp` @ 2d478061): a shaft from the tail stopping at the head's
//! base center, plus a FILLED head whose size scales from the thickness -
//! base width `ARROW_HEAD_WIDTH + 2t`, tip-to-base height
//! `ARROW_HEAD_HEIGHT + 2t`, both clamped to the shaft length for short
//! arrows. `arrowStyle` selects the straight triangular head or the curved
//! (quadratic-notch) swept head; `reverseArrow` flips the head to the press
//! point. BORROW-MODIFIED: Flameshot's short-arrow integer-division hack
//! (`setLength(thickness / 4)`, their own "looks not very bad" comment) is
//! replaced by the clean `head_len = min(height + 2t, len)` clamp, and the
//! style/reverse config is captured PER OBJECT at commit time (Flameshot
//! re-reads `reverseArrow` at every paint, retro-flipping existing arrows).

use serde::{Deserialize, Serialize};

use super::{Color, PaintSink, Point, Rect, ToolObject, ToolObjectData};
use crate::config::ArrowStyle;

/// Head base width before thickness scaling (`arrowtool.cpp` `ArrowWidth`).
pub const ARROW_HEAD_WIDTH: f32 = 10.0;
/// Head height before thickness scaling (`arrowtool.cpp` `ArrowHeight`).
pub const ARROW_HEAD_HEIGHT: f32 = 18.0;
/// The curved head's notch depth factor (`baseDistance * 0.45` in
/// `getCurvedArrowHead`).
const NOTCH_FACTOR: f32 = 0.45;
/// The curved head's control-point inset factor (`halfWidth * 0.25`).
const CONTROL_FACTOR: f32 = 0.25;
/// The curved shaft's extension into the concave notch
/// (`getCurvedArrowShaft`'s `overlap`), hiding the seam.
const CURVE_SHAFT_OVERLAP: f32 = 1.0;
/// Curve flattening tolerance in scene px (chord error far below the
/// renderer's antialiasing footprint).
const FLATTEN_TOLERANCE: f32 = 0.05;
/// Flattening segment bounds (a head is at most ~60px across; 64 segments
/// keep the chord error under a thousandth of a pixel).
const FLATTEN_MIN: f32 = 4.0;
const FLATTEN_MAX: f32 = 64.0;

/// An arrow annotation: a shaft plus a filled, thickness-scaled head.
///
/// The head sits at [`ArrowObject::to`] by default; [`ArrowObject::reverse`]
/// moves it to [`ArrowObject::from`] (the `reverseArrow` config parity).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArrowObject {
    /// Tail of the arrow (the press point as drawn).
    pub from: Point,
    /// Tip of the arrow (the release point as drawn).
    pub to: Point,
    /// Line color.
    pub color: Color,
    /// Line thickness.
    pub thickness: f32,
    /// Head style (`[tools.arrow].style`, captured at commit).
    #[serde(default)]
    pub style: ArrowStyle,
    /// Head at `from` instead of `to` (`[tools.arrow].reverse`, captured at
    /// commit).
    #[serde(default)]
    pub reverse: bool,
}

impl ArrowObject {
    /// Creates a straight, non-reversed arrow annotation.
    #[must_use]
    pub fn new(from: Point, to: Point, color: Color, thickness: f32) -> Self {
        Self {
            from,
            to,
            color,
            thickness,
            style: ArrowStyle::default(),
            reverse: false,
        }
    }

    /// Sets the head style (builder).
    #[must_use]
    pub fn with_style(mut self, style: ArrowStyle) -> Self {
        self.style = style;
        self
    }

    /// Sets the head reversal (builder).
    #[must_use]
    pub fn with_reverse(mut self, reverse: bool) -> Self {
        self.reverse = reverse;
        self
    }

    /// Decomposes the arrow into `(shaft_end, head_polygon)`: the shaft runs
    /// from the tail to `shaft_end` (already extended into the notch for the
    /// curved style), and the head is the filled polygon with the tip first.
    ///
    /// `None` when the arrow is degenerate (zero or non-finite length).
    #[must_use]
    pub fn shaft_and_head(&self) -> Option<(Point, Vec<Point>)> {
        let (base, tip) = if self.reverse {
            (self.to, self.from)
        } else {
            (self.from, self.to)
        };
        let (dx, dy) = (tip.x - base.x, tip.y - base.y);
        let len = (dx * dx + dy * dy).sqrt();
        if !len.is_finite() || len <= 0.0 {
            return None;
        }
        let t = self.thickness.max(0.0);
        let (dir_x, dir_y) = (dx / len, dy / len);
        let head_len = (ARROW_HEAD_HEIGHT + 2.0 * t).min(len);
        let shaft_len = len - head_len;
        let center = Point::new(base.x + dir_x * shaft_len, base.y + dir_y * shaft_len);
        let half_width = ARROW_HEAD_WIDTH * 0.5 + t;
        let (nx, ny) = (-dir_y, dir_x);
        let offset = |point: Point, axis: (f32, f32), amount: f32| {
            Point::new(point.x + axis.0 * amount, point.y + axis.1 * amount)
        };
        let base_left = offset(center, (nx, ny), half_width);
        let base_right = offset(center, (nx, ny), -half_width);
        match self.style {
            ArrowStyle::Straight => Some((center, vec![tip, base_left, base_right])),
            ArrowStyle::Curved => {
                let notch_depth = (head_len * NOTCH_FACTOR).min(half_width);
                let notch = offset(center, (dir_x, dir_y), notch_depth);
                let left_control = offset(center, (nx, ny), half_width * CONTROL_FACTOR);
                let right_control = offset(center, (nx, ny), -half_width * CONTROL_FACTOR);
                let mut polygon = vec![tip, base_left];
                flatten_quad(&mut polygon, base_left, left_control, notch);
                flatten_quad(&mut polygon, notch, right_control, base_right);
                let shaft_end = offset(
                    center,
                    (dir_x, dir_y),
                    (notch_depth + CURVE_SHAFT_OVERLAP).min(head_len),
                );
                Some((shaft_end, polygon))
            }
        }
    }
}

/// Appends the flattened quadratic bezier `p0 -> p1` (control `c`), excluding
/// the start point (adaptive segment count from the control-point accel, the
/// standard chord-error bound).
fn flatten_quad(out: &mut Vec<Point>, p0: Point, c: Point, p1: Point) {
    let accel = ((p0.x - 2.0 * c.x + p1.x).powi(2) + (p0.y - 2.0 * c.y + p1.y).powi(2)).sqrt();
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the value is clamped to [FLATTEN_MIN, FLATTEN_MAX] before the cast"
    )]
    let segments = (accel / (8.0 * FLATTEN_TOLERANCE))
        .sqrt()
        .ceil()
        .clamp(FLATTEN_MIN, FLATTEN_MAX) as usize;
    for step in 1..=segments {
        #[expect(
            clippy::cast_precision_loss,
            reason = "segment counts are bounded by FLATTEN_MAX, far under 2^24"
        )]
        let (t, mt) = {
            let t = step as f32 / segments as f32;
            (t, 1.0 - t)
        };
        out.push(Point::new(
            mt * mt * p0.x + 2.0 * mt * t * c.x + t * t * p1.x,
            mt * mt * p0.y + 2.0 * mt * t * c.y + t * t * p1.y,
        ));
    }
}

impl ToolObject for ArrowObject {
    fn type_id(&self) -> &'static str {
        "arrow"
    }

    fn bounding_rect(&self) -> Rect {
        // Exact ink bounds: the shaft endpoints and the filled head corners,
        // grown by the shaft's half-width stroke ink (the F27
        // `ArrowTool::boundingRect` walks the head path the same way).
        let Some((_, head)) = self.shaft_and_head() else {
            return Rect::from_points(self.from, self.to);
        };
        let mut min_x = self.from.x.min(self.to.x);
        let mut min_y = self.from.y.min(self.to.y);
        let mut max_x = self.from.x.max(self.to.x);
        let mut max_y = self.from.y.max(self.to.y);
        for point in &head {
            min_x = min_x.min(point.x);
            min_y = min_y.min(point.y);
            max_x = max_x.max(point.x);
            max_y = max_y.max(point.y);
        }
        let grow = self.thickness.max(0.0) * 0.5;
        Rect::new(
            min_x - grow,
            min_y - grow,
            (max_x - min_x) + grow * 2.0,
            (max_y - min_y) + grow * 2.0,
        )
    }

    fn paint(&self, sink: &mut dyn PaintSink) {
        let Some((shaft_end, head)) = self.shaft_and_head() else {
            return;
        };
        let base = if self.reverse { self.to } else { self.from };
        let t = self.thickness.max(0.0);
        if (shaft_end.x - base.x).abs() + (shaft_end.y - base.y).abs() > 0.0 {
            sink.draw_line(base, shaft_end, self.color, t);
        }
        sink.fill_polygon(&head, self.color);
    }

    fn to_data(&self) -> ToolObjectData {
        ToolObjectData::Arrow(self.clone())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

    use super::*;
    use crate::scene::test_support::{Call, RecSink};

    const RED: Color = Color::new(255, 0, 0, 255);

    fn arrow(from: (f32, f32), to: (f32, f32), thickness: f32) -> ArrowObject {
        ArrowObject::new(
            Point::new(from.0, from.1),
            Point::new(to.0, to.1),
            RED,
            thickness,
        )
    }

    #[test]
    fn straight_head_is_a_thickness_scaled_triangle_at_the_tip() {
        // F27 math: head height 18 + 2t, half base width 5 + t.
        let arrow = arrow((0.0, 0.0), (100.0, 0.0), 2.0);
        let (shaft_end, head) = arrow.shaft_and_head().expect("non-degenerate");
        assert_eq!(shaft_end, Point::new(78.0, 0.0), "100 - (18 + 4)");
        assert_eq!(head.len(), 3);
        assert_eq!(head[0], Point::new(100.0, 0.0), "tip first");
        assert_eq!(head[1], Point::new(78.0, 7.0), "5 + t half width");
        assert_eq!(head[2], Point::new(78.0, -7.0));
    }

    #[test]
    fn reverse_moves_the_head_to_the_press_point() {
        let arrow = arrow((0.0, 0.0), (100.0, 0.0), 2.0).with_reverse(true);
        let (shaft_end, head) = arrow.shaft_and_head().expect("non-degenerate");
        assert_eq!(head[0], Point::new(0.0, 0.0), "tip at from");
        assert_eq!(shaft_end, Point::new(22.0, 0.0), "shaft from to inward");
    }

    #[test]
    fn curved_head_carries_the_quadratic_notch() {
        let arrow = arrow((0.0, 0.0), (100.0, 0.0), 2.0).with_style(ArrowStyle::Curved);
        let (shaft_end, head) = arrow.shaft_and_head().expect("non-degenerate");
        // notch depth = min(22 * 0.45, 7) = 7 -> the shaft extends to 78 + 8.
        assert_eq!(shaft_end, Point::new(86.0, 0.0));
        assert_eq!(head[0], Point::new(100.0, 0.0));
        assert_eq!(head[1], Point::new(78.0, 7.0));
        assert!(
            head.iter().any(|p| *p == Point::new(85.0, 0.0)),
            "the notch point lies on the polygon"
        );
        assert_eq!(
            head.last().copied(),
            Some(Point::new(78.0, -7.0)),
            "flattening ends at base_right"
        );
        assert!(head.len() > 3, "both quads flattened into segments");
    }

    #[test]
    fn short_arrows_clamp_the_head_to_the_shaft() {
        // len 10 < 18 + 2t: the head consumes the whole arrow, no shaft.
        let arrow = arrow((0.0, 0.0), (10.0, 0.0), 2.0);
        let (shaft_end, head) = arrow.shaft_and_head().expect("non-degenerate");
        assert_eq!(shaft_end, Point::new(0.0, 0.0));
        assert_eq!(head[0], Point::new(10.0, 0.0));
        assert_eq!(head[1], Point::new(0.0, 7.0));
        let mut sink = RecSink::default();
        arrow.paint(&mut sink);
        assert_eq!(
            sink.calls,
            vec![Call::Polygon(head, RED)],
            "no zero-length shaft line under the head"
        );
    }

    #[test]
    fn degenerate_arrows_paint_nothing_but_stay_finite() {
        let arrow = arrow((5.0, 5.0), (5.0, 5.0), 2.0);
        assert!(arrow.shaft_and_head().is_none());
        let mut sink = RecSink::default();
        arrow.paint(&mut sink);
        assert!(sink.calls.is_empty());
        assert_eq!(arrow.bounding_rect(), Rect::new(5.0, 5.0, 0.0, 0.0));
    }

    #[test]
    fn bounding_rect_covers_the_head_ink() {
        let arrow = arrow((0.0, 0.0), (100.0, 0.0), 2.0);
        // Head corners (78, +-7), tip (100, 0), tail (0, 0), grown by t/2 = 1.
        assert_eq!(arrow.bounding_rect(), Rect::new(-1.0, -8.0, 102.0, 16.0));
        let reversed = arrow.clone().with_reverse(true);
        assert_eq!(
            reversed.bounding_rect(),
            Rect::new(-1.0, -8.0, 102.0, 16.0),
            "reversal mirrors within the same bounds"
        );
    }

    #[test]
    fn paint_emits_the_shaft_line_then_the_filled_head() {
        let arrow = arrow((0.0, 0.0), (100.0, 0.0), 2.0);
        let mut sink = RecSink::default();
        arrow.paint(&mut sink);
        let [
            Call::Line(from, to, color, width),
            Call::Polygon(head, head_color),
        ] = &sink.calls[..]
        else {
            panic!("shaft + head, got {:?}", sink.calls);
        };
        assert_eq!(*from, Point::new(0.0, 0.0));
        assert_eq!(*to, Point::new(78.0, 0.0));
        assert_eq!(*color, RED);
        assert_eq!(*width, 2.0);
        assert_eq!(head[0], Point::new(100.0, 0.0));
        assert_eq!(*head_color, RED);
    }

    #[test]
    fn serde_defaults_keep_old_data_straight_and_forward() {
        let legacy = r"
            from = { x = 0.0, y = 0.0 }
            to = { x = 10.0, y = 0.0 }
            color = { r = 255, g = 0, b = 0, a = 255 }
            thickness = 2.0
        ";
        let parsed: ArrowObject = toml::from_str(legacy).expect("legacy shape");
        assert_eq!(parsed.style, ArrowStyle::Straight);
        assert!(!parsed.reverse);
        let styled = parsed
            .clone()
            .with_style(ArrowStyle::Curved)
            .with_reverse(true);
        let text = toml::to_string(&styled).expect("serialize");
        assert!(text.contains("style = \"curved\"") && text.contains("reverse = true"));
        let reloaded: ArrowObject = toml::from_str(&text).expect("reload");
        assert_eq!(reloaded, styled);
    }
}
