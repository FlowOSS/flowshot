//! The numbered step-counter bubble (draft F27 circlecount spec).
//!
//! Clean-room reimplementation of the Flameshot `CircleCountTool` paint
//! geometry (`src/tools/circlecount/circlecounttool.cpp` @ 2d478061):
//!
//! - a FILLED bubble, radius = dispatched tool size + [`COUNTER_THICKNESS_OFFSET`]
//!   (their `bubble_size = size() + THICKNESS_OFFSET`);
//! - the optional outline ring (`drawCircleCounterOutline`): a
//!   [`COUNTER_PADDING`]-wide anti-contrast ring around the bubble plus
//!   contrast hairlines on both circles (their cosmetic-pen ellipse pair);
//! - a BOLD contrast digit (white on dark fills, black on light - their
//!   `ColorUtils::colorIsDark` pick) shrunk-to-fit inside a `radius`-wide
//!   box (their `boundingRect` decrement loop, solved directly);
//! - the press-drag aim pointer: a filled triangle, apex at the drag
//!   target, base on the bubble diameter perpendicular to the drag (their
//!   `normalVector().setLength(bubble_size)` + point-mirrored `p2`), painted
//!   only while the target is farther than one radius from the center
//!   (their `line.length() > bubble_size` gate).

use serde::{Deserialize, Serialize};

use super::{Color, LabelStyle, PaintSink, Point, Rect, ToolObject, ToolObjectData};

/// Ring padding around the bubble (`circlecounttool.cpp` `PADDING_VALUE`).
pub const COUNTER_PADDING: f32 = 2.0;
/// Radius added to the dispatched tool size (`circlecounttool.cpp`
/// `THICKNESS_OFFSET`: `bubble_size = size() + THICKNESS_OFFSET`).
pub const COUNTER_THICKNESS_OFFSET: f32 = 15.0;

/// The outline hairline width (Qt's cosmetic pen renders 1px).
const OUTLINE_WIDTH: f32 = 1.0;
/// Average digit advance as a fraction of the font size (the codebase-wide
/// estimate - the shaped text itself is measured by the renderer).
const LABEL_ADVANCE: f32 = 0.6;
/// Luma at or below which a color reads as dark (`colorutils.cpp`
/// `getColorLuma`: `0.30r + 0.59g + 0.11b`, threshold `0.5`).
const DARK_LUMA: f32 = 0.5;

/// Whether `color` reads as dark (Flameshot `ColorUtils::colorIsDark`).
#[must_use]
pub fn color_is_dark(color: Color) -> bool {
    let luma = 0.30 * f32::from(color.r) + 0.59 * f32::from(color.g) + 0.11 * f32::from(color.b);
    luma <= DARK_LUMA * 255.0
}

/// The digit ink color: white on dark fills, black on light (the
/// `contrastColor` pick in `CircleCountTool::process`).
#[must_use]
pub fn contrast_color(color: Color) -> Color {
    if color_is_dark(color) {
        Color::new(255, 255, 255, 255)
    } else {
        Color::new(0, 0, 0, 255)
    }
}

/// The outline-ring fill: the opposite of [`contrast_color`] (the
/// `antiContrastColor` pick in `CircleCountTool::process`).
#[must_use]
pub fn anti_contrast_color(color: Color) -> Color {
    if color_is_dark(color) {
        Color::new(0, 0, 0, 255)
    } else {
        Color::new(255, 255, 255, 255)
    }
}

/// The digit font size for `chars` characters in a bubble of `radius`.
///
/// Flameshot starts at `bubble_size` (= `radius`) and decrements until the
/// measured width fits the `bubble_size`-wide text box; with the estimated
/// advance the loop's fixed point is `radius / max(1, chars * 0.6)` (one
/// digit keeps the full radius, two shrink to `radius / 1.2`, three to
/// `radius / 1.8`).
#[must_use]
pub fn label_font_size(radius: f32, chars: usize) -> f32 {
    let chars = u32::try_from(chars).unwrap_or(u32::MAX);
    #[expect(clippy::cast_precision_loss, reason = "digit counts are tiny")]
    let factor = (chars as f32 * LABEL_ADVANCE).max(1.0);
    radius / factor
}

/// A numbered step-counter annotation (filled bubble + centered digit,
/// optional outline ring, optional drag-aim pointer).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CounterObject {
    /// Bubble center.
    pub center: Point,
    /// Bubble radius (Flameshot `bubble_size = size() + THICKNESS_OFFSET`).
    pub radius: f32,
    /// Bubble fill color (`[editor].draw_color`).
    pub color: Color,
    /// Displayed number. `0` means unassigned; the scene's `add_object`
    /// numbers such counters with the max+1 rule.
    pub count: u32,
    /// Whether the outline ring paints (`[tools.counter].outline`, captured
    /// per object at commit - the `drawCircleCounterOutline` parity).
    #[serde(default = "default_outline")]
    pub outline: bool,
    /// The drag-aim target (Flameshot's released `points().second`): when
    /// farther than `radius` from `center`, the pointer triangle paints
    /// toward it.
    #[serde(default)]
    pub pointer: Option<Point>,
}

/// The serde default for [`CounterObject::outline`] (data from before the
/// field existed carried the always-on outline paint).
const fn default_outline() -> bool {
    true
}

impl CounterObject {
    /// Creates a counter annotation. Pass `count = 0` to let the scene
    /// auto-number it on add. The outline defaults on, no drag pointer.
    #[must_use]
    pub fn new(center: Point, radius: f32, color: Color, count: u32) -> Self {
        Self {
            center,
            radius,
            color,
            count,
            outline: true,
            pointer: None,
        }
    }

    /// Sets the outline-ring flag (`[tools.counter].outline`).
    #[must_use]
    pub const fn with_outline(mut self, outline: bool) -> Self {
        self.outline = outline;
        self
    }

    /// Sets the drag-aim pointer target.
    #[must_use]
    pub const fn with_pointer(mut self, pointer: Option<Point>) -> Self {
        self.pointer = pointer;
        self
    }

    /// The aim-pointer triangle: apex at the drag target, base on the
    /// bubble's diameter perpendicular to the drag.
    ///
    /// Flameshot walks `center -> p1 -> target -> p2 -> center` with `p1`
    /// the drag vector's normal scaled to `bubble_size` and `p2` its mirror
    /// through the center - a path enclosing exactly this triangle (the
    /// center lies on the base). `None` when no pointer is set or the
    /// target sits within one radius of the center (their paint gate).
    #[must_use]
    pub fn pointer_triangle(&self) -> Option<[Point; 3]> {
        let target = self.pointer?;
        let dx = target.x - self.center.x;
        let dy = target.y - self.center.y;
        let len = dx.hypot(dy);
        if !len.is_finite() || len <= self.radius {
            return None;
        }
        let scale = self.radius / len;
        let p1 = Point::new(self.center.x + dy * scale, self.center.y - dx * scale);
        let p2 = Point::new(self.center.x - dy * scale, self.center.y + dx * scale);
        Some([p1, target, p2])
    }
}

impl ToolObject for CounterObject {
    fn type_id(&self) -> &'static str {
        "counter"
    }

    fn bounding_rect(&self) -> Rect {
        // Flameshot `boundingRect`: the bubble grown by PADDING_VALUE,
        // unioned with the drag target.
        let pad = self.radius + COUNTER_PADDING;
        let bubble = Rect::new(
            self.center.x - pad,
            self.center.y - pad,
            pad * 2.0,
            pad * 2.0,
        );
        let Some(target) = self.pointer else {
            return bubble;
        };
        Rect::from_points(
            Point::new(bubble.x.min(target.x), bubble.y.min(target.y)),
            Point::new(
                (bubble.x + bubble.width).max(target.x),
                (bubble.y + bubble.height).max(target.y),
            ),
        )
    }

    fn paint(&self, sink: &mut dyn PaintSink) {
        // The whole bubble shares the fill's alpha so the translucent hover
        // preview reads as one object (Flameshot's painter-wide
        // `setOpacity(0.35)` in `paintMousePreview`).
        let alpha = self.color.a;
        let contrast = contrast_color(self.color).with_alpha(alpha);
        if let Some(triangle) = self.pointer_triangle() {
            sink.fill_polygon(&triangle, self.color);
        }
        if self.outline {
            let ring = self.radius + COUNTER_PADDING;
            let rect = Rect::new(
                self.center.x - ring,
                self.center.y - ring,
                ring * 2.0,
                ring * 2.0,
            );
            sink.fill_ellipse(rect, anti_contrast_color(self.color).with_alpha(alpha));
            sink.stroke_ellipse(rect, contrast, OUTLINE_WIDTH);
        }
        let bubble = Rect::new(
            self.center.x - self.radius,
            self.center.y - self.radius,
            self.radius * 2.0,
            self.radius * 2.0,
        );
        sink.fill_ellipse(bubble, self.color);
        if self.outline {
            sink.stroke_ellipse(bubble, contrast, OUTLINE_WIDTH);
        }
        let label = self.count.to_string();
        sink.draw_text_centered(
            self.center,
            &label,
            LabelStyle {
                font_size: label_font_size(self.radius, label.chars().count()),
                color: contrast,
                bold: true,
            },
        );
    }

    fn count(&self) -> Option<u32> {
        Some(self.count)
    }

    fn set_count(&mut self, count: u32) {
        self.count = count;
    }

    fn to_data(&self) -> ToolObjectData {
        ToolObjectData::Counter(self.clone())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

    use super::*;
    use crate::scene::test_support::{Call, RecSink};

    const RED: Color = Color::new(255, 0, 0, 255);
    const WHITE: Color = Color::new(255, 255, 255, 255);
    const BLACK: Color = Color::new(0, 0, 0, 255);

    fn bubble(radius: f32) -> CounterObject {
        CounterObject::new(Point::new(100.0, 100.0), radius, RED, 1)
    }

    #[test]
    fn pointer_triangle_anchors_base_perpendicular_to_the_drag() {
        // Drag straight right from (100,100) to (160,100), radius 16: the
        // base is the vertical diameter (100,84)-(100,116), apex at target.
        let counter = bubble(16.0).with_pointer(Some(Point::new(160.0, 100.0)));
        let [p1, apex, p2] = counter.pointer_triangle().expect("visible pointer");
        assert_eq!(p1, Point::new(100.0, 84.0));
        assert_eq!(p2, Point::new(100.0, 116.0));
        assert_eq!(apex, Point::new(160.0, 100.0));
    }

    #[test]
    fn pointer_triangle_normalizes_diagonal_drags() {
        // 45-degree drag: the base endpoints stay one radius from the
        // center, on the perpendicular.
        let counter = bubble(10.0).with_pointer(Some(Point::new(150.0, 150.0)));
        let [p1, _, p2] = counter.pointer_triangle().expect("visible pointer");
        let s = 10.0 / std::f32::consts::SQRT_2;
        assert_eq!(p1, Point::new(100.0 + s, 100.0 - s));
        assert_eq!(p2, Point::new(100.0 - s, 100.0 + s));
        for p in [p1, p2] {
            let d = ((p.x - 100.0).powi(2) + (p.y - 100.0).powi(2)).sqrt();
            assert!((d - 10.0).abs() < 1e-5, "base point at radius: {d}");
        }
    }

    #[test]
    fn pointer_hidden_within_one_radius() {
        // Flameshot gate: line.length() > bubble_size, else no pointer.
        assert_eq!(bubble(16.0).pointer_triangle(), None);
        assert_eq!(
            bubble(16.0)
                .with_pointer(Some(Point::new(110.0, 100.0)))
                .pointer_triangle(),
            None,
            "target inside the bubble"
        );
        assert_eq!(
            bubble(16.0)
                .with_pointer(Some(Point::new(116.0, 100.0)))
                .pointer_triangle(),
            None,
            "target exactly at the radius (gate is strict >)"
        );
        assert!(
            bubble(16.0)
                .with_pointer(Some(Point::new(116.1, 100.0)))
                .pointer_triangle()
                .is_some(),
            "target just past the radius"
        );
    }

    #[test]
    fn label_font_size_shrinks_to_fit_the_radius_box() {
        let fits = |chars: usize, expected: f32| {
            let got = label_font_size(16.0, chars);
            assert!((got - expected).abs() < 1e-4, "chars={chars}: {got}");
        };
        fits(1, 16.0);
        fits(2, 16.0 / 1.2);
        fits(3, 16.0 / 1.8);
    }

    #[test]
    fn contrast_follows_the_luma_rule() {
        assert!(color_is_dark(RED), "red luma 0.30 <= 0.5");
        assert!(color_is_dark(BLACK));
        assert!(!color_is_dark(WHITE));
        assert!(
            !color_is_dark(Color::new(255, 255, 0, 255)),
            "yellow luma 0.89"
        );
        assert_eq!(contrast_color(RED), WHITE);
        assert_eq!(contrast_color(Color::new(255, 255, 0, 255)), BLACK);
        assert_eq!(anti_contrast_color(RED), BLACK);
        assert_eq!(anti_contrast_color(Color::new(255, 255, 0, 255)), WHITE);
    }

    #[test]
    fn paint_with_outline_emits_ring_bubble_and_centered_bold_label() {
        let mut sink = RecSink::default();
        bubble(16.0).paint(&mut sink);
        let ring = Rect::new(82.0, 82.0, 36.0, 36.0);
        let bubble_rect = Rect::new(84.0, 84.0, 32.0, 32.0);
        assert_eq!(
            sink.calls,
            vec![
                Call::FillEllipse(ring, BLACK),
                Call::StrokeEllipse(ring, WHITE, 1.0),
                Call::FillEllipse(bubble_rect, RED),
                Call::StrokeEllipse(bubble_rect, WHITE, 1.0),
                Call::CenteredText(
                    Point::new(100.0, 100.0),
                    "1".to_owned(),
                    LabelStyle {
                        font_size: 16.0,
                        color: WHITE,
                        bold: true
                    }
                ),
            ]
        );
    }

    #[test]
    fn paint_without_outline_emits_bubble_and_label_only() {
        let mut sink = RecSink::default();
        bubble(16.0).with_outline(false).paint(&mut sink);
        assert_eq!(
            sink.calls,
            vec![
                Call::FillEllipse(Rect::new(84.0, 84.0, 32.0, 32.0), RED),
                Call::CenteredText(
                    Point::new(100.0, 100.0),
                    "1".to_owned(),
                    LabelStyle {
                        font_size: 16.0,
                        color: WHITE,
                        bold: true
                    }
                ),
            ]
        );
    }

    #[test]
    fn paint_with_drag_emits_the_pointer_first() {
        let mut sink = RecSink::default();
        let counter = CounterObject::new(Point::new(100.0, 100.0), 16.0, RED, 7)
            .with_outline(false)
            .with_pointer(Some(Point::new(160.0, 100.0)));
        counter.paint(&mut sink);
        assert_eq!(
            sink.calls[0],
            Call::Polygon(
                vec![
                    Point::new(100.0, 84.0),
                    Point::new(160.0, 100.0),
                    Point::new(100.0, 116.0)
                ],
                RED
            )
        );
        assert!(
            matches!(sink.calls.last(), Some(Call::CenteredText(_, label, _)) if label == "7"),
            "the digit still paints: {:?}",
            sink.calls.last()
        );
    }

    #[test]
    fn translucent_fill_translucents_the_whole_bubble() {
        let mut sink = RecSink::default();
        let preview =
            CounterObject::new(Point::new(100.0, 100.0), 16.0, Color::new(255, 0, 0, 89), 3);
        preview.paint(&mut sink);
        let Call::FillEllipse(_, ring_color) = sink.calls[0] else {
            panic!("ring fill first");
        };
        assert_eq!(ring_color, Color::new(0, 0, 0, 89));
        let Call::CenteredText(_, _, style) = sink.calls.last().unwrap() else {
            panic!("label last");
        };
        assert_eq!(style.color, Color::new(255, 255, 255, 89));
    }

    #[test]
    fn bounds_cover_ring_and_drag_target() {
        assert_eq!(
            bubble(16.0).bounding_rect(),
            Rect::new(82.0, 82.0, 36.0, 36.0)
        );
        let dragged = bubble(16.0).with_pointer(Some(Point::new(160.0, 90.0)));
        assert_eq!(
            dragged.bounding_rect(),
            Rect::new(82.0, 82.0, 160.0 - 82.0, 118.0 - 82.0)
        );
    }
}
