//! Resize and bounds math for the drag machine (draft F27).
//!
//! The resize mirrors Flameshot's `parentMouseMoveEvent` exactly: Ctrl
//! constrains the aspect ratio with the per-handle formulas (edge drags move
//! the bottom/right companion edge - "this behavior feels natural"), Shift
//! mirrors the resize around the start center (`topLeft += dTL - dBR`,
//! L370-376), and crossing through the opposite edge flips the active
//! handle (`getProperSide`) so the drag keeps tracking the cursor. The
//! `FlowShot` modification is the 10x10 minimum enforced on every result.
//!
//! [`fit_into_bounds`] is the explicit layout clamp for rects that can
//! overshoot even though every cursor position arrives layout-clamped
//! (moves with a grab offset, mirrored resizes, min-size expansion).

use flowshot_core::geometry::{Logical, LogicalPoint, LogicalRect};

use super::drag::DragMove;
use super::hit::Handle;

/// The four rect edges (left, top, right, bottom) as plain f64s - the
/// resize formulas' working representation.
type Edges = (f64, f64, f64, f64);

/// The absolute resize from `start` through `handle` to the cursor: naive
/// edges, Ctrl aspect override, Shift mirror, flip detection, normalize,
/// minimum enforcement, bounds fit. Returns the rect and the (possibly
/// flipped) active handle.
pub(super) fn resize_rect(
    start: LogicalRect,
    handle: Handle,
    input: &DragMove<'_>,
) -> (LogicalRect, Handle) {
    let start_edges = (start.x.0, start.y.0, start.right().0, start.bottom().0);
    let (left, top, right, bottom) = start_edges;
    let (mut nl, mut nt, mut nr, mut nb) = naive_edges(handle, start_edges, input.at);
    if input.ctrl {
        (nl, nt, nr, nb) = aspect_edges(handle, aspect_of(start), (nl, nt, nr, nb), input.at);
    }
    if !(nl.is_finite() && nt.is_finite() && nr.is_finite() && nb.is_finite()) {
        // A degenerate aspect computation (0/0) poisoned an edge; the naive
        // drag is always finite (the cursor position is validated upstream).
        (nl, nt, nr, nb) = naive_edges(handle, start_edges, input.at);
    }
    if input.shift {
        // Mirror resize (F27 L370-376): topLeft += dTL - dBR and
        // bottomRight += dBR - dTL, symmetric around the start center.
        let (dtl_x, dtl_y) = (nl - left, nt - top);
        let (dbr_x, dbr_y) = (nr - right, nb - bottom);
        nl = left + dtl_x - dbr_x;
        nt = top + dtl_y - dbr_y;
        nr = right + dbr_x - dtl_x;
        nb = bottom + dbr_y - dtl_y;
    }
    // Flip the active handle when the drag crossed through the opposite
    // edge (F27 `getProperSide`), then normalize.
    let handle = handle.flipped(nr < nl, nb < nt);
    let (x0, x1) = ordered(nl, nr);
    let (y0, y1) = ordered(nt, nb);
    (nl, nt, nr, nb) = (x0, y0, x1, y1);
    let min = input.metrics.min_side;
    if input.shift {
        // The mirror keeps the start center: enforce the minimum symmetrically.
        if nr - nl < min {
            let center = f64::midpoint(nl, nr);
            nl = center - min / 2.0;
            nr = center + min / 2.0;
        }
        if nb - nt < min {
            let center = f64::midpoint(nt, nb);
            nt = center - min / 2.0;
            nb = center + min / 2.0;
        }
    } else {
        if handle.moves_right() && nr - nl < min {
            nr = nl + min;
        }
        if handle.moves_left() && nr - nl < min {
            nl = nr - min;
        }
        if handle.moves_bottom() && nb - nt < min {
            nb = nt + min;
        }
        if handle.moves_top() && nb - nt < min {
            nt = nb - min;
        }
    }
    (
        fit_into_bounds(
            LogicalRect::from_raw(nl, nt, nr - nl, nb - nt),
            input.bounds,
        ),
        handle,
    )
}

/// The edges the cursor drags directly (no modifiers).
fn naive_edges(handle: Handle, start: Edges, at: LogicalPoint) -> Edges {
    let (mut nl, mut nt, mut nr, mut nb) = start;
    let (px, py) = (at.x.0, at.y.0);
    match handle {
        Handle::TopLeft => {
            nl = px;
            nt = py;
        }
        Handle::TopRight => {
            nr = px;
            nt = py;
        }
        Handle::BottomLeft => {
            nl = px;
            nb = py;
        }
        Handle::BottomRight => {
            nr = px;
            nb = py;
        }
        Handle::Left => nl = px,
        Handle::Right => nr = px,
        Handle::Top => nt = py,
        Handle::Bottom => nb = py,
    }
    (nl, nt, nr, nb)
}

/// The Ctrl aspect-constrain override (F27 `parentMouseMoveEvent` formulas
/// verbatim): corners pick the dominant axis, edge drags move the bottom or
/// right companion edge to compensate. The naive tuple carries the START
/// edges the formulas need: a handle never modifies the edges its own
/// aspect formula reads (corners read the two opposite edges, edge handles
/// read the perpendicular pair). The four inputs are the formula's literal
/// independent terms (which handle, the locked ratio, the current edges,
/// the cursor) - no further grouping exists in the spec math.
fn aspect_edges(handle: Handle, aspect: f64, naive: Edges, at: LogicalPoint) -> Edges {
    let (mut nl, mut nt, mut nr, mut nb) = naive;
    let (px, py) = (at.x.0, at.y.0);
    match handle {
        Handle::TopLeft => {
            if (nr - px) / (nb - py) > aspect {
                nt = nb - (nr - px) / aspect;
            } else {
                nl = nr - (nb - py) * aspect;
            }
        }
        Handle::BottomRight => {
            if (px - nl) / (py - nt) > aspect {
                nb = nt + (px - nl) / aspect;
            } else {
                nr = nl + (py - nt) * aspect;
            }
        }
        Handle::TopRight => {
            if (px - nl) / (nb - py) > aspect {
                nt = nb - (px - nl) / aspect;
            } else {
                nr = nl + (nb - py) * aspect;
            }
        }
        Handle::BottomLeft => {
            if (nr - px) / (py - nt) > aspect {
                nb = nt + (nr - px) / aspect;
            } else {
                nl = nr - (py - nt) * aspect;
            }
        }
        Handle::Left => nb = nt + (nr - px) / aspect,
        Handle::Right => nb = nt + (px - nl) / aspect,
        Handle::Top => nr = nl + (nb - py) * aspect,
        Handle::Bottom => nr = nl + (py - nt) * aspect,
    }
    (nl, nt, nr, nb)
}

/// Shifts `rect` into `bounds` preserving its size when it fits; clamps it
/// to the bounds (intersection semantics) when it cannot fit. `None` bounds
/// (an empty layout) pass the rect through.
pub(crate) fn fit_into_bounds(rect: LogicalRect, bounds: Option<LogicalRect>) -> LogicalRect {
    let Some(bounds) = bounds else {
        return rect;
    };
    let (x, width) = fit_axis(rect.x, rect.width, bounds.x, bounds.width);
    let (y, height) = fit_axis(rect.y, rect.height, bounds.y, bounds.height);
    LogicalRect::new(x, y, width, height)
}

fn fit_axis(
    origin: Logical,
    size: Logical,
    bounds_origin: Logical,
    bounds_size: Logical,
) -> (Logical, Logical) {
    if size.0 > bounds_size.0 {
        return (bounds_origin, bounds_size);
    }
    let bounds_end = bounds_origin.0 + bounds_size.0;
    if origin.0 < bounds_origin.0 {
        (bounds_origin, size)
    } else if origin.0 + size.0 > bounds_end {
        (Logical(bounds_end - size.0), size)
    } else {
        (origin, size)
    }
}

/// Width/height at press time (the Ctrl constraint target); a degenerate
/// height falls back to 1.0 (Flameshot's `m_aspectRatio` initial value).
pub(super) fn aspect_of(rect: LogicalRect) -> f64 {
    if rect.height.0 > 0.0 {
        rect.width.0 / rect.height.0
    } else {
        1.0
    }
}

/// Sorts two edge coordinates into (low, high).
pub(super) fn ordered(a: f64, b: f64) -> (f64, f64) {
    if a <= b { (a, b) } else { (b, a) }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::float_cmp,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic
    )]

    use flowshot_core::geometry::LogicalPoint;
    use flowshot_core::tokens::DesignTokens;

    use super::super::metrics::SelectionMetrics;
    use super::*;

    fn logical(x: f64, y: f64, w: f64, h: f64) -> LogicalRect {
        LogicalRect::from_raw(x, y, w, h)
    }

    #[test]
    fn fit_preserves_size_inside_and_clamps_oversize() {
        let b = Some(logical(0.0, 0.0, 1920.0, 1080.0));
        // Fits: shifted back in, size preserved.
        let fitted = fit_into_bounds(logical(-50.0, 10.0, 100.0, 100.0), b);
        assert_eq!(fitted, logical(0.0, 10.0, 100.0, 100.0));
        // Wider than the bounds: clamped to the bounds exactly.
        let fitted = fit_into_bounds(logical(-100.0, 0.0, 5000.0, 10.0), b);
        assert_eq!(fitted, logical(0.0, 0.0, 1920.0, 10.0));
        // No bounds: pass-through.
        let fitted = fit_into_bounds(logical(5.0, 5.0, 10.0, 10.0), None);
        assert_eq!(fitted, logical(5.0, 5.0, 10.0, 10.0));
    }

    #[test]
    fn aspect_of_guards_the_degenerate_height() {
        assert_eq!(aspect_of(logical(0.0, 0.0, 200.0, 100.0)), 2.0);
        assert_eq!(aspect_of(logical(0.0, 0.0, 200.0, 0.0)), 1.0);
    }

    #[test]
    fn resize_rect_applies_the_aspect_dominant_axis() {
        let metrics = SelectionMetrics::from_tokens(&DesignTokens::default());
        let start = logical(500.0, 400.0, 200.0, 100.0); // aspect 2.0
        let input = DragMove {
            at: LogicalPoint::from_raw(750.0, 510.0),
            shift: false,
            ctrl: true,
            metrics: &metrics,
            bounds: Some(logical(0.0, 0.0, 4480.0, 1440.0)),
        };
        // (px-l)/(py-t) = 250/110 > 2 -> width dominates: h = 250/2.
        let (rect, handle) = resize_rect(start, Handle::BottomRight, &input);
        assert_eq!(rect, logical(500.0, 400.0, 250.0, 125.0));
        assert_eq!(handle, Handle::BottomRight);
    }
}
