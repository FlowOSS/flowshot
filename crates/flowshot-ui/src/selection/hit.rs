//! Handle geometry and hit-testing (plan todo 16, draft F27).
//!
//! Eight handles: four corners and four edges. Hit priority is
//! corners > edges > center (F27: `selectionwidget.cpp` `getMouseSide`
//! checks the four corner areas, then the four edge strips, then the rect
//! interior). The corner areas are squares of side `handle_area` centered on
//! the corners; the edge strips run BETWEEN the corner areas (F27:
//! `updateAreas` - `m_LArea = QRect(m_TLArea.bottomLeft(), m_BLArea.topRight())`
//! etc.), so on a small selection the edge strips degenerate to empty and
//! only corners + interior remain hittable - Flameshot parity.

use flowshot_core::geometry::{Logical, LogicalPoint, LogicalRect};

use super::metrics::SelectionMetrics;

/// One of the eight resize handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Handle {
    /// Top-left corner.
    TopLeft,
    /// Top edge.
    Top,
    /// Top-right corner.
    TopRight,
    /// Right edge.
    Right,
    /// Bottom-right corner.
    BottomRight,
    /// Bottom edge.
    Bottom,
    /// Bottom-left corner.
    BottomLeft,
    /// Left edge.
    Left,
}

impl Handle {
    /// All eight handles, corners first (the hit-test and paint order).
    pub const ALL: [Self; 8] = [
        Self::TopLeft,
        Self::TopRight,
        Self::BottomLeft,
        Self::BottomRight,
        Self::Left,
        Self::Top,
        Self::Right,
        Self::Bottom,
    ];

    /// Whether dragging this handle moves the left edge.
    #[must_use]
    pub const fn moves_left(self) -> bool {
        matches!(self, Self::TopLeft | Self::Left | Self::BottomLeft)
    }

    /// Whether dragging this handle moves the right edge.
    #[must_use]
    pub const fn moves_right(self) -> bool {
        matches!(self, Self::TopRight | Self::Right | Self::BottomRight)
    }

    /// Whether dragging this handle moves the top edge.
    #[must_use]
    pub const fn moves_top(self) -> bool {
        matches!(self, Self::TopLeft | Self::Top | Self::TopRight)
    }

    /// Whether dragging this handle moves the bottom edge.
    #[must_use]
    pub const fn moves_bottom(self) -> bool {
        matches!(self, Self::BottomLeft | Self::Bottom | Self::BottomRight)
    }

    /// Whether this is a corner handle (moves two edges).
    #[must_use]
    pub const fn is_corner(self) -> bool {
        matches!(
            self,
            Self::TopLeft | Self::TopRight | Self::BottomRight | Self::BottomLeft
        )
    }

    /// The handle after the rect flipped through an edge, mirroring the
    /// active side so the drag keeps feeling natural (F27: `getProperSide`
    /// XORs the LEFT/RIGHT bits when `right < left` and the TOP/BOTTOM bits
    /// when `bottom < top`).
    #[must_use]
    pub const fn flipped(self, horizontal: bool, vertical: bool) -> Self {
        let h = match self {
            Self::TopLeft => Self::TopRight,
            Self::TopRight => Self::TopLeft,
            Self::BottomLeft => Self::BottomRight,
            Self::BottomRight => Self::BottomLeft,
            Self::Left => Self::Right,
            Self::Right => Self::Left,
            Self::Top => Self::Top,
            Self::Bottom => Self::Bottom,
        };
        let source = if horizontal { h } else { self };
        let v = match source {
            Self::TopLeft => Self::BottomLeft,
            Self::BottomLeft => Self::TopLeft,
            Self::TopRight => Self::BottomRight,
            Self::BottomRight => Self::TopRight,
            Self::Top => Self::Bottom,
            Self::Bottom => Self::Top,
            Self::Left => Self::Left,
            Self::Right => Self::Right,
        };
        if vertical { v } else { source }
    }

    /// The grip anchor point (the handle's visual center) on `rect`.
    #[must_use]
    pub fn anchor(self, rect: LogicalRect) -> LogicalPoint {
        let (x, y) = match self {
            Self::TopLeft => (rect.x, rect.y),
            Self::Top => (center(rect.x, rect.right()), rect.y),
            Self::TopRight => (rect.right(), rect.y),
            Self::Right => (rect.right(), center(rect.y, rect.bottom())),
            Self::BottomRight => (rect.right(), rect.bottom()),
            Self::Bottom => (center(rect.x, rect.right()), rect.bottom()),
            Self::BottomLeft => (rect.x, rect.bottom()),
            Self::Left => (rect.x, center(rect.y, rect.bottom())),
        };
        LogicalPoint::new(x, y)
    }

    /// The square/strip hit area of this handle on `rect` (F27
    /// `updateAreas`: corner squares of side `handle_area` centered on the
    /// corners; edge strips of the same width running between them).
    #[must_use]
    pub fn area(self, rect: LogicalRect, metrics: &SelectionMetrics) -> LogicalRect {
        let side = metrics.handle_area;
        let half = side / 2.0;
        let (l, t, r, b) = (rect.x.0, rect.y.0, rect.right().0, rect.bottom().0);
        match self {
            Self::TopLeft => corner(l, t, half, side),
            Self::TopRight => corner(r, t, half, side),
            Self::BottomLeft => corner(l, b, half, side),
            Self::BottomRight => corner(r, b, half, side),
            Self::Left => LogicalRect::from_raw(l - half, t + half, side, b - t - side),
            Self::Right => LogicalRect::from_raw(r - half, t + half, side, b - t - side),
            Self::Top => LogicalRect::from_raw(l + half, t - half, r - l - side, side),
            Self::Bottom => LogicalRect::from_raw(l + half, b - half, r - l - side, side),
        }
    }
}

/// What a pointer position resolves to on (or around) a selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitZone {
    /// One of the eight handles (corners take priority over edges).
    Handle(Handle),
    /// The selection interior (drag moves it).
    Inside,
    /// Neither the selection nor its handles (drag creates a new one).
    Outside,
}

/// Resolves `point` against `rect`: corners > edges > center (F27
/// `getMouseSide` order: TL, TR, BL, BR, L, T, R, B, CENTER).
#[must_use]
pub fn hit_zone(rect: LogicalRect, point: LogicalPoint, metrics: &SelectionMetrics) -> HitZone {
    for handle in Handle::ALL {
        if handle.area(rect, metrics).contains_point(point) {
            return HitZone::Handle(handle);
        }
    }
    if rect.contains_point(point) {
        return HitZone::Inside;
    }
    HitZone::Outside
}

fn corner(center_x: f64, center_y: f64, half: f64, side: f64) -> LogicalRect {
    LogicalRect::from_raw(center_x - half, center_y - half, side, side)
}

fn center(a: Logical, b: Logical) -> Logical {
    Logical(f64::midpoint(a.0, b.0))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use flowshot_core::tokens::DesignTokens;

    use super::*;

    fn metrics() -> SelectionMetrics {
        SelectionMetrics::from_tokens(&DesignTokens::default())
    }

    /// 100x80 selection at (50, 40); handle area = 14 * 1.2 * 2.2 * 0.6
    /// = 22.176, half = 11.088.
    fn rect() -> LogicalRect {
        LogicalRect::from_raw(50.0, 40.0, 100.0, 80.0)
    }

    fn point(x: f64, y: f64) -> LogicalPoint {
        LogicalPoint::from_raw(x, y)
    }

    #[test]
    fn corners_win_over_edges_and_center() {
        let m = metrics();
        // Exactly on the top-left corner: both the TL corner area and the
        // top/left strips could claim neighborhoods - corners check first.
        assert_eq!(
            hit_zone(rect(), point(50.0, 40.0), &m),
            HitZone::Handle(Handle::TopLeft)
        );
        assert_eq!(
            hit_zone(rect(), point(150.0, 40.0), &m),
            HitZone::Handle(Handle::TopRight)
        );
        assert_eq!(
            hit_zone(rect(), point(50.0, 120.0), &m),
            HitZone::Handle(Handle::BottomLeft)
        );
        assert_eq!(
            hit_zone(rect(), point(150.0, 120.0), &m),
            HitZone::Handle(Handle::BottomRight)
        );
    }

    #[test]
    fn edge_strips_hit_between_corner_areas() {
        let m = metrics();
        // Top edge midpoint: inside the top strip, outside every corner area.
        assert_eq!(
            hit_zone(rect(), point(100.0, 40.0), &m),
            HitZone::Handle(Handle::Top)
        );
        assert_eq!(
            hit_zone(rect(), point(100.0, 120.0), &m),
            HitZone::Handle(Handle::Bottom)
        );
        assert_eq!(
            hit_zone(rect(), point(50.0, 80.0), &m),
            HitZone::Handle(Handle::Left)
        );
        assert_eq!(
            hit_zone(rect(), point(150.0, 80.0), &m),
            HitZone::Handle(Handle::Right)
        );
    }

    #[test]
    fn inside_and_outside_resolve() {
        let m = metrics();
        assert_eq!(hit_zone(rect(), point(100.0, 80.0), &m), HitZone::Inside);
        assert_eq!(hit_zone(rect(), point(10.0, 10.0), &m), HitZone::Outside);
        // Just outside the handle area band (half = 11.088): x = 50 - 12 is
        // beyond the left strip, y = 80 is between the corner areas.
        assert_eq!(hit_zone(rect(), point(37.0, 80.0), &m), HitZone::Outside);
    }

    #[test]
    fn handle_areas_match_the_flameshot_construction() {
        let m = metrics();
        let half = m.handle_area / 2.0;
        let tl = Handle::TopLeft.area(rect(), &m);
        assert_eq!(
            tl,
            LogicalRect::from_raw(50.0 - half, 40.0 - half, m.handle_area, m.handle_area)
        );
        // Left strip spans vertically BETWEEN the corner areas.
        let left = Handle::Left.area(rect(), &m);
        assert_eq!(left.y.0, 40.0 + half);
        assert_eq!(left.height.0, 80.0 - 2.0 * half);
        assert_eq!(left.width.0, m.handle_area);
    }

    #[test]
    fn tiny_selection_leaves_edge_strips_empty() {
        let m = metrics();
        // 10x10 rect < handle area (22.176): edge strips degenerate; the
        // corner areas cover everything around the rect.
        let tiny = LogicalRect::from_raw(0.0, 0.0, 10.0, 10.0);
        assert!(Handle::Left.area(tiny, &m).is_empty());
        assert_eq!(
            hit_zone(tiny, point(5.0, 5.0), &m),
            HitZone::Handle(Handle::TopLeft)
        );
    }

    #[test]
    fn anchors_are_corner_and_edge_midpoints() {
        assert_eq!(Handle::TopLeft.anchor(rect()), point(50.0, 40.0));
        assert_eq!(Handle::Top.anchor(rect()), point(100.0, 40.0));
        assert_eq!(Handle::Right.anchor(rect()), point(150.0, 80.0));
        assert_eq!(Handle::Bottom.anchor(rect()), point(100.0, 120.0));
    }

    #[test]
    fn flips_mirror_the_active_handle() {
        assert_eq!(Handle::BottomRight.flipped(true, false), Handle::BottomLeft);
        assert_eq!(Handle::BottomRight.flipped(false, true), Handle::TopRight);
        assert_eq!(Handle::BottomRight.flipped(true, true), Handle::TopLeft);
        assert_eq!(Handle::Left.flipped(true, false), Handle::Right);
        assert_eq!(Handle::Top.flipped(false, true), Handle::Bottom);
        assert_eq!(Handle::Top.flipped(true, false), Handle::Top);
        assert_eq!(Handle::TopLeft.flipped(true, true), Handle::BottomRight);
    }

    #[test]
    fn edge_predicates_partition_the_handles() {
        for handle in Handle::ALL {
            let vertical_edges = u8::from(handle.moves_left()) + u8::from(handle.moves_right());
            let horizontal_edges = u8::from(handle.moves_top()) + u8::from(handle.moves_bottom());
            let total = vertical_edges + horizontal_edges;
            assert_eq!(
                total,
                if handle.is_corner() { 2 } else { 1 },
                "{handle:?} moves one edge (side handles) or two (corners)"
            );
            assert_eq!(
                handle.is_corner(),
                vertical_edges == 1 && horizontal_edges == 1
            );
        }
    }
}
