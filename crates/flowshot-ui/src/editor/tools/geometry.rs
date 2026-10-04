//! Shared stroke geometry for the shape tools (draft F27).
//!
//! Two clean-room Flameshot patterns power all seven tools:
//!
//! - the two-point stroke (`abstracttwopointtool.cpp`): a press/move/release
//!   point pair with the Ctrl drag conventions applied AT USE time (paint and
//!   commit see the same constrained endpoint, so pressing Ctrl mid-drag
//!   updates the live preview without waiting for the next motion - the
//!   `EditorView.modifiers` paint contract). [`adjusted`] is the
//!   `adjustedVector` snap math: H/V/45deg for the orthogonal+diagonal tools
//!   (line/arrow/marker), 45deg-only for the diagonal tools (rectangle
//!   square-lock, ellipse circle-lock).
//! - the path stroke (`abstractpathtool.cpp`): accumulated freehand points,
//!   valid from the second point on, simplified at commit with
//!   Ramer-Douglas-Peucker at the plan's [`RDP_EPSILON`] (0.5px).

use flowshot_core::geometry::{LogicalPoint, LogicalRect};
use flowshot_core::scene::{PaintSink, Point as ScenePoint, Rect as SceneRect};

use crate::render::f32_from_f64;

use super::super::tool::EditorContext;

/// The pencil simplification epsilon in scene px ("point
/// simplification on drawEnd - Ramer-Douglas-Peucker epsilon = 0.5px").
pub const RDP_EPSILON: f32 = 0.5;

/// The mouse-preview dot padding (F27 `mousePreviewRect`: a `toolSize + 2`
/// square centered on the cursor).
const PREVIEW_PADDING: f32 = 2.0;

/// The Ctrl drag convention a two-point tool applies (F27
/// `m_supportsOrthogonalAdj` / `m_supportsDiagonalAdj`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Constrain {
    /// No adjustment (invert).
    Free,
    /// H/V/45deg snap (line, arrow, marker).
    OrthogonalDiagonal,
    /// 45deg-only snap = square/circle lock (rectangle, ellipse).
    DiagonalOnly,
}

/// Converts a global logical point into scene space (f32).
pub(super) fn scene_point(at: LogicalPoint) -> ScenePoint {
    ScenePoint::new(f32_from_f64(at.x.0), f32_from_f64(at.y.0))
}

/// The two-point stroke state: raw press/move points; the constrain is
/// applied at use time with the live modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(super) struct TwoPoint {
    from: Option<ScenePoint>,
    to: Option<ScenePoint>,
}

impl TwoPoint {
    /// Opens the stroke at `at` (F27 `drawStart`: both points at the press).
    pub(super) fn start(&mut self, at: LogicalPoint) {
        let point = scene_point(at);
        self.from = Some(point);
        self.to = Some(point);
    }

    /// Extends the stroke (F27 `drawMove`; the release point finalizes).
    pub(super) fn extend(&mut self, at: LogicalPoint) {
        self.to = Some(scene_point(at));
    }

    /// Whether a stroke is open.
    pub(super) fn drawing(&self) -> bool {
        self.from.is_some()
    }

    /// Drops the stroke state (after commit, so the hover preview never
    /// repaints the committed shape).
    pub(super) fn clear(&mut self) {
        self.from = None;
        self.to = None;
    }

    /// The constrained endpoint pair, or `None` before `start`. The pair is
    /// degenerate (equal points) exactly when the drag had zero length - the
    /// commit-validity rule of the shape tools.
    pub(super) fn endpoints(
        &self,
        ctx: &EditorContext<'_>,
        mode: Constrain,
    ) -> Option<(ScenePoint, ScenePoint)> {
        let (from, to) = (self.from?, self.to?);
        let to = if ctx.modifiers.control_key() {
            let (dx, dy) = adjusted(to.x - from.x, to.y - from.y, mode);
            ScenePoint::new(from.x + dx, from.y + dy)
        } else {
            to
        };
        Some((from, to))
    }

    /// The RAW (unconstrained) endpoint bounds - the damage rect of the
    /// in-progress shape (`bounding_rect` has no modifier snapshot).
    pub(super) fn raw_bounds(&self) -> Option<SceneRect> {
        points_bounds(&[self.from?, self.to?])
    }

    /// Ends the stroke: the constrained endpoint pair when the drag was
    /// non-degenerate (the shape tools' zero-length rule), clearing the
    /// state so the hover preview never repaints the committed shape.
    pub(super) fn finish(
        &mut self,
        ctx: &EditorContext<'_>,
        mode: Constrain,
    ) -> Option<(ScenePoint, ScenePoint)> {
        let endpoints = self.endpoints(ctx, mode).filter(|(from, to)| from != to);
        self.clear();
        endpoints
    }
}

/// The F27 `adjustedVector` snap (clean-room from `abstracttwopointtool.cpp`
/// @ 2d478061): the delta snapped to the nearest 45deg increment
/// (`OrthogonalDiagonal`: axis snaps zero the off-axis component, diagonals
/// average) or to the nearest diagonal (`DiagonalOnly`).
fn adjusted(dx: f32, dy: f32, mode: Constrain) -> (f32, f32) {
    const ADJ_UNIT: f32 = std::f32::consts::FRAC_PI_4;
    if mode == Constrain::Free {
        return (dx, dy);
    }
    // Screen y grows downward; the snap angle measures against -dy (the
    // mathematical convention), exactly like the F27 source.
    let angle = (-dy).atan2(dx);
    let diagonal = |dx: f32, dy: f32, up: bool| {
        let n = if up {
            (dx - dy) * 0.5
        } else {
            f32::midpoint(dx, dy)
        };
        if up { (n, -n) } else { (n, n) }
    };
    match mode {
        Constrain::Free => (dx, dy),
        Constrain::OrthogonalDiagonal => {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "angle / 45deg lies in [-4, 4] and is rounded first"
            )]
            let dir = ((angle / ADJ_UNIT).round() as i32).rem_euclid(4);
            match dir {
                0 => (dx, 0.0),
                2 => (0.0, dy),
                1 => diagonal(dx, dy, true),
                _ => diagonal(dx, dy, false),
            }
        }
        Constrain::DiagonalOnly => {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "the quotient lies in [-3, 2] and is rounded first"
            )]
            let dir = (((angle - ADJ_UNIT) / (2.0 * ADJ_UNIT)).round() as i32).rem_euclid(2);
            if dir == 0 {
                diagonal(dx, dy, true)
            } else {
                diagonal(dx, dy, false)
            }
        }
    }
}

/// Ramer-Douglas-Peucker polyline simplification (iterative; keeps the first
/// and last point and every point farther than `epsilon` from its chord).
pub(super) fn simplify(points: &[ScenePoint], epsilon: f32) -> Vec<ScenePoint> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let last = points.len() - 1;
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[last] = true;
    let mut stack = vec![(0usize, last)];
    while let Some((start, end)) = stack.pop() {
        let mut farthest = None;
        let mut max_distance = epsilon;
        for index in start + 1..end {
            let distance =
                perpendicular_distance(points[index], points[start], points[end]).max(0.0);
            if distance.is_finite() && distance > max_distance {
                max_distance = distance;
                farthest = Some(index);
            }
        }
        if let Some(index) = farthest {
            keep[index] = true;
            stack.push((start, index));
            stack.push((index, end));
        }
    }
    points
        .iter()
        .zip(&keep)
        .filter_map(|(point, kept)| kept.then_some(*point))
        .collect()
}

/// The perpendicular distance from `point` to the segment `a`-`b` (the
/// degenerate segment yields the plain point distance).
fn perpendicular_distance(point: ScenePoint, a: ScenePoint, b: ScenePoint) -> f32 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let length_sq = dx * dx + dy * dy;
    if length_sq <= 0.0 {
        return ((point.x - a.x).powi(2) + (point.y - a.y).powi(2)).sqrt();
    }
    ((point.x - a.x) * dy - (point.y - a.y) * dx).abs() / length_sq.sqrt()
}

/// The F27 mouse preview: a dot of `size + 2` scene px centered on the
/// cursor (`mousePreviewRect` parity), painted in `color`.
pub(super) fn paint_preview_dot(
    sink: &mut dyn PaintSink,
    ctx: &EditorContext<'_>,
    color: flowshot_core::scene::Color,
    size: f32,
) {
    let center = scene_point(ctx.mouse);
    let radius = f32::midpoint(size, PREVIEW_PADDING);
    sink.fill_ellipse(
        SceneRect::new(
            center.x - radius,
            center.y - radius,
            radius * 2.0,
            radius * 2.0,
        ),
        color,
    );
}

/// The scene-space bounds of a point set (in-progress shape damage rect).
pub(super) fn points_bounds(points: &[ScenePoint]) -> Option<SceneRect> {
    let first = points.first()?;
    let mut rect = SceneRect::new(first.x, first.y, 0.0, 0.0);
    for point in points {
        let min_x = rect.x.min(point.x);
        let min_y = rect.y.min(point.y);
        let max_x = (rect.x + rect.width).max(point.x);
        let max_y = (rect.y + rect.height).max(point.y);
        rect = SceneRect::new(min_x, min_y, max_x - min_x, max_y - min_y);
    }
    Some(rect)
}

/// Converts a scene rect into the logical rect the [`Tool`](super::super::tool::Tool)
/// trait reports (scene space IS global logical px - a lossless f32 -> f64
/// widening).
pub(super) fn logical(bounds: SceneRect) -> LogicalRect {
    LogicalRect::from_raw(
        f64::from(bounds.x),
        f64::from(bounds.y),
        f64::from(bounds.width),
        f64::from(bounds.height),
    )
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::float_cmp,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::cast_precision_loss
    )]

    use super::*;

    fn point(x: f32, y: f32) -> ScenePoint {
        ScenePoint::new(x, y)
    }

    #[test]
    fn orthogonal_diagonal_snap_covers_the_four_directions() {
        // Nearest 45deg increment, F27 adjustedVector math (screen y down).
        assert_eq!(
            adjusted(10.0, 1.0, Constrain::OrthogonalDiagonal),
            (10.0, 0.0)
        );
        assert_eq!(
            adjusted(1.0, 10.0, Constrain::OrthogonalDiagonal),
            (0.0, 10.0)
        );
        assert_eq!(
            adjusted(10.0, 5.0, Constrain::OrthogonalDiagonal),
            (7.5, 7.5)
        );
        assert_eq!(
            adjusted(10.0, -5.0, Constrain::OrthogonalDiagonal),
            (7.5, -7.5)
        );
        assert_eq!(
            adjusted(-10.0, 5.0, Constrain::OrthogonalDiagonal),
            (-7.5, 7.5)
        );
        assert_eq!(
            adjusted(-10.0, -5.0, Constrain::OrthogonalDiagonal),
            (-7.5, -7.5)
        );
    }

    #[test]
    fn diagonal_only_snap_is_the_square_lock() {
        // The rect/ellipse convention: |dx| == |dy| after the snap.
        for (dx, dy) in [(10.0, 1.0), (1.0, 10.0), (10.0, 5.0), (-8.0, 3.0)] {
            let (nx, ny) = adjusted(dx, dy, Constrain::DiagonalOnly);
            assert_eq!(nx.abs(), ny.abs(), "({dx},{dy}) -> ({nx},{ny})");
        }
        assert_eq!(adjusted(10.0, -5.0, Constrain::DiagonalOnly), (7.5, -7.5));
        assert_eq!(adjusted(10.0, 5.0, Constrain::DiagonalOnly), (7.5, 7.5));
    }

    #[test]
    fn free_constrain_is_the_identity() {
        assert_eq!(adjusted(3.0, -7.0, Constrain::Free), (3.0, -7.0));
    }

    #[test]
    fn rdp_drops_collinear_points_and_keeps_corners() {
        let mut line: Vec<ScenePoint> = (0..=20).map(|i| point(i as f32, 0.0)).collect();
        assert_eq!(
            simplify(&line, RDP_EPSILON),
            vec![point(0.0, 0.0), point(20.0, 0.0)]
        );
        // A corner survives; a 0.4px wobble does not (epsilon 0.5).
        line.push(point(20.0, 0.4));
        let zigzag = vec![
            point(0.0, 0.0),
            point(5.0, 0.4),
            point(10.0, 0.0),
            point(15.0, 8.0),
            point(20.0, 0.0),
        ];
        assert_eq!(
            simplify(&zigzag, RDP_EPSILON),
            vec![
                point(0.0, 0.0),
                point(10.0, 0.0),
                point(15.0, 8.0),
                point(20.0, 0.0)
            ]
        );
        // Short inputs pass through.
        assert_eq!(
            simplify(&line[..2], RDP_EPSILON),
            vec![point(0.0, 0.0), point(1.0, 0.0)]
        );
        assert!(simplify(&[], RDP_EPSILON).is_empty());
    }

    #[test]
    fn rdp_handles_a_degenerate_all_equal_path() {
        let points = vec![point(3.0, 3.0); 8];
        assert_eq!(
            simplify(&points, RDP_EPSILON),
            vec![point(3.0, 3.0), point(3.0, 3.0)]
        );
    }

    #[test]
    fn bounds_cover_the_point_set() {
        assert_eq!(points_bounds(&[]), None);
        let bounds = points_bounds(&[point(5.0, -2.0), point(-1.0, 4.0), point(0.0, 0.0)]);
        assert_eq!(bounds, Some(SceneRect::new(-1.0, -2.0, 6.0, 6.0)));
    }
}
