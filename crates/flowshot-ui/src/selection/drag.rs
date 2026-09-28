//! The drag state machine: create, move, resize (draft F27).
//!
//! Clean-room reimplementation of the Flameshot `SelectionWidget` mouse
//! semantics (`selectionwidget.cpp` @2d478061), with two `FlowShot`
//! modifications from the spec:
//!
//! - creation is gated behind the 3px manhattan threshold (a click is never
//!   a selection; Flameshot creates on the first motion event), and
//! - every rect enforces the 10x10 logical-px minimum (Flameshot: 1x1).
//!
//! The resize/clamp math itself lives in [`super::resize`]: Ctrl constrains
//! the aspect ratio, Shift mirrors around the start center, and crossing
//! through the opposite edge flips the active handle - the exact Flameshot
//! formulas. All positions arriving here are already clamped to the layout
//! by the router; the resize module's bounds fit is the explicit clamp for
//! rects that can still overshoot (moves with a grab offset, mirrored
//! resizes, min-size expansion).

use flowshot_core::geometry::{LogicalPoint, LogicalRect};

use super::hit::{Handle, HitZone, hit_zone};
use super::metrics::SelectionMetrics;
use super::resize::{fit_into_bounds, ordered, resize_rect};

/// The active pointer drag.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Drag {
    /// A left press that has not (yet) exceeded the drag threshold; becomes
    /// a creation drag once it does (latched).
    Creating {
        /// The press position (global logical).
        origin: LogicalPoint,
        /// Whether the threshold was exceeded.
        committed: bool,
    },
    /// An inside drag translating the selection.
    Moving {
        /// Selection origin minus the grab position (constant during the
        /// drag, so the rect never jumps to the cursor).
        grab: LogicalPoint,
    },
    /// A handle drag resizing the selection.
    Resizing {
        /// The active handle (flips when the drag crosses through).
        handle: Handle,
        /// The rect at press time (resize is absolute from here; the Ctrl
        /// aspect target derives from it).
        start: LogicalRect,
    },
}

/// Everything one drag-motion step needs.
#[derive(Debug, Clone, Copy)]
pub(super) struct DragMove<'a> {
    /// The pointer position (global logical, layout-clamped).
    pub at: LogicalPoint,
    /// Shift held (mirror resize).
    pub shift: bool,
    /// Ctrl held (aspect constrain).
    pub ctrl: bool,
    /// Token-derived sizes (threshold, minimum).
    pub metrics: &'a SelectionMetrics,
    /// The layout union bounds (the clamp target).
    pub bounds: Option<LogicalRect>,
}

/// The outcome of one drag-motion step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum DragStep {
    /// Nothing changed yet (creation below the threshold, or a stale drag).
    Pending(Drag),
    /// The drag produced a new selection rect.
    Changed {
        /// The (possibly updated) drag state.
        drag: Drag,
        /// The new selection rect.
        rect: LogicalRect,
    },
}

/// Starts a drag for a left press at `at` against the current selection:
/// handle -> resize, inside -> move, anywhere else -> create.
pub(super) fn begin(
    rect: Option<LogicalRect>,
    at: LogicalPoint,
    metrics: &SelectionMetrics,
) -> Drag {
    match rect {
        Some(current) => match hit_zone(current, at, metrics) {
            HitZone::Handle(handle) => Drag::Resizing {
                handle,
                start: current,
            },
            HitZone::Inside => Drag::Moving {
                grab: LogicalPoint::new(current.x - at.x, current.y - at.y),
            },
            HitZone::Outside => Drag::Creating {
                origin: at,
                committed: false,
            },
        },
        None => Drag::Creating {
            origin: at,
            committed: false,
        },
    }
}

/// Advances the drag to the pointer position `input.at`.
pub(super) fn advance(drag: Drag, rect: Option<LogicalRect>, input: &DragMove<'_>) -> DragStep {
    match drag {
        Drag::Creating { origin, committed } => {
            let committed = committed || manhattan(origin, input.at) > input.metrics.drag_threshold;
            if committed {
                DragStep::Changed {
                    drag: Drag::Creating {
                        origin,
                        committed: true,
                    },
                    rect: creation_rect(origin, input.at, input.metrics.min_side, input.bounds),
                }
            } else {
                DragStep::Pending(Drag::Creating {
                    origin,
                    committed: false,
                })
            }
        }
        Drag::Moving { grab } => match rect {
            Some(current) => {
                let origin = LogicalPoint::new(grab.x + input.at.x, grab.y + input.at.y);
                let moved = LogicalRect::from_parts(origin, current.size());
                DragStep::Changed {
                    drag,
                    rect: fit_into_bounds(moved, input.bounds),
                }
            }
            None => DragStep::Pending(drag),
        },
        Drag::Resizing { handle, start } => {
            let (rect, handle) = resize_rect(start, handle, input);
            DragStep::Changed {
                drag: Drag::Resizing { handle, start },
                rect,
            }
        }
    }
}

/// Completes a left release. An uncommitted creation is a CLICK: it clears
/// the selection when it landed outside (Flameshot
/// `parentMouseReleaseEvent` hides the selection on an outside release) and
/// keeps it on an inside/handle click. Committed drags keep their rect.
pub(super) fn release(
    drag: Drag,
    rect: Option<LogicalRect>,
    at: LogicalPoint,
    metrics: &SelectionMetrics,
) -> Option<LogicalRect> {
    match drag {
        Drag::Creating {
            committed: false, ..
        } => rect.filter(|current| hit_zone(*current, at, metrics) != HitZone::Outside),
        Drag::Creating {
            committed: true, ..
        }
        | Drag::Moving { .. }
        | Drag::Resizing { .. } => rect,
    }
}

/// The creation rect between `origin` and the cursor, normalized, with the
/// minimum enforced away from the ORIGIN corner (the press point stays put)
/// and the result fitted into the layout bounds.
fn creation_rect(
    origin: LogicalPoint,
    at: LogicalPoint,
    min: f64,
    bounds: Option<LogicalRect>,
) -> LogicalRect {
    let (mut left, mut right) = ordered(origin.x.0, at.x.0);
    let (mut top, mut bottom) = ordered(origin.y.0, at.y.0);
    if right - left < min {
        if origin.x.0 <= at.x.0 {
            right = left + min;
        } else {
            left = right - min;
        }
    }
    if bottom - top < min {
        if origin.y.0 <= at.y.0 {
            bottom = top + min;
        } else {
            top = bottom - min;
        }
    }
    fit_into_bounds(
        LogicalRect::from_raw(left, top, right - left, bottom - top),
        bounds,
    )
}

pub(super) fn manhattan(a: LogicalPoint, b: LogicalPoint) -> f64 {
    (a.x.0 - b.x.0).abs() + (a.y.0 - b.y.0).abs()
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::float_cmp,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::unnecessary_wraps
    )]

    use flowshot_core::tokens::DesignTokens;

    use super::*;

    fn metrics() -> SelectionMetrics {
        SelectionMetrics::from_tokens(&DesignTokens::default())
    }

    fn bounds() -> Option<LogicalRect> {
        Some(LogicalRect::from_raw(0.0, 0.0, 1920.0, 1080.0))
    }

    fn input(at: (f64, f64), shift: bool, ctrl: bool, m: &SelectionMetrics) -> DragMove<'_> {
        DragMove {
            at: LogicalPoint::from_raw(at.0, at.1),
            shift,
            ctrl,
            metrics: m,
            bounds: bounds(),
        }
    }

    fn logical(x: f64, y: f64, w: f64, h: f64) -> LogicalRect {
        LogicalRect::from_raw(x, y, w, h)
    }

    #[test]
    fn creation_below_threshold_stays_pending() {
        let m = metrics();
        let drag = begin(None, LogicalPoint::from_raw(100.0, 100.0), &m);
        // Manhattan 3.0 is NOT > 3.0 (the code value: strictly greater).
        let step = advance(drag, None, &input((103.0, 100.0), false, false, &m));
        assert!(matches!(
            step,
            DragStep::Pending(Drag::Creating {
                committed: false,
                ..
            })
        ));
        let step = advance(drag, None, &input((104.0, 100.0), false, false, &m));
        let DragStep::Changed { rect, .. } = step else {
            panic!("threshold exceeded must create");
        };
        // 4x0 attempt -> min 10x10 anchored at the origin corner.
        assert_eq!(rect, logical(100.0, 100.0, 10.0, 10.0));
    }

    #[test]
    fn creation_normalizes_and_anchors_the_origin_corner() {
        let m = metrics();
        let origin = LogicalPoint::from_raw(100.0, 100.0);
        let drag = begin(None, origin, &m);
        // Drag up-left: the origin stays the BOTTOM-right corner.
        let step = advance(drag, None, &input((50.0, 40.0), false, false, &m));
        let DragStep::Changed { rect, .. } = step else {
            panic!("committed");
        };
        assert_eq!(rect, logical(50.0, 40.0, 50.0, 60.0));
    }

    #[test]
    fn creation_clamps_to_bounds() {
        let m = metrics();
        let drag = begin(None, LogicalPoint::from_raw(1915.0, 1075.0), &m);
        let step = advance(drag, None, &input((1919.0, 1079.0), false, false, &m));
        let DragStep::Changed { rect: r, .. } = step else {
            panic!("committed");
        };
        // 4x4 attempt -> min 10x10 anchored at the origin corner would run
        // past (1920, 1080) -> shifted back in, size preserved.
        assert_eq!(r, logical(1910.0, 1070.0, 10.0, 10.0));
    }

    #[test]
    fn move_translates_by_the_grab_offset_and_slides_at_bounds() {
        let m = metrics();
        let current = logical(100.0, 100.0, 200.0, 150.0);
        let drag = begin(Some(current), LogicalPoint::from_raw(150.0, 120.0), &m);
        assert!(matches!(drag, Drag::Moving { .. }));
        let step = advance(
            drag,
            Some(current),
            &input((250.0, 220.0), false, false, &m),
        );
        let DragStep::Changed { rect: r, .. } = step else {
            panic!("moved");
        };
        assert_eq!(r, logical(200.0, 200.0, 200.0, 150.0));
        // Far past the right edge: slides, size preserved.
        let step = advance(
            drag,
            Some(current),
            &input((5000.0, 120.0), false, false, &m),
        );
        let DragStep::Changed { rect: r, .. } = step else {
            panic!("moved");
        };
        assert_eq!(r, logical(1720.0, 100.0, 200.0, 150.0));
    }

    #[test]
    fn bottom_right_resize_tracks_the_cursor() {
        let m = metrics();
        let start = logical(100.0, 100.0, 100.0, 100.0);
        let drag = begin(Some(start), LogicalPoint::from_raw(200.0, 200.0), &m);
        assert!(matches!(
            drag,
            Drag::Resizing {
                handle: Handle::BottomRight,
                ..
            }
        ));
        let step = advance(drag, Some(start), &input((300.0, 250.0), false, false, &m));
        let DragStep::Changed { rect: r, .. } = step else {
            panic!("resized");
        };
        assert_eq!(r, logical(100.0, 100.0, 200.0, 150.0));
    }

    #[test]
    fn resize_enforces_the_minimum_at_the_fixed_edge() {
        let m = metrics();
        let start = logical(100.0, 100.0, 100.0, 100.0);
        let drag = begin(Some(start), LogicalPoint::from_raw(200.0, 200.0), &m);
        // Shrink to 5x5: clamps to 10x10 anchored at the fixed top-left.
        let step = advance(drag, Some(start), &input((105.0, 105.0), false, false, &m));
        let DragStep::Changed { rect: r, .. } = step else {
            panic!("resized");
        };
        assert_eq!(r, logical(100.0, 100.0, 10.0, 10.0));
    }

    #[test]
    fn shift_mirrors_around_the_start_center() {
        let m = metrics();
        let start = logical(100.0, 100.0, 100.0, 100.0);
        let drag = begin(Some(start), LogicalPoint::from_raw(200.0, 200.0), &m);
        // BR drag to (220, 220) with Shift: dBR = (20, 20), dTL = 0 ->
        // topLeft -= 20, bottomRight += 20 (symmetric around (150, 150)).
        let step = advance(drag, Some(start), &input((220.0, 220.0), true, false, &m));
        let DragStep::Changed { rect: r, .. } = step else {
            panic!("resized");
        };
        assert_eq!(r, logical(80.0, 80.0, 140.0, 140.0));
    }

    #[test]
    fn ctrl_constrains_the_aspect_on_corners() {
        let m = metrics();
        let start = logical(100.0, 100.0, 200.0, 100.0); // aspect 2.0
        let drag = begin(Some(start), LogicalPoint::from_raw(300.0, 200.0), &m);
        // BR to (350, 210): (px-l)/(py-t) = 250/110 = 2.27 > 2 -> width
        // dominates: bottom = 100 + 250/2 = 225.
        let step = advance(drag, Some(start), &input((350.0, 210.0), false, true, &m));
        let DragStep::Changed { rect: r, .. } = step else {
            panic!("resized");
        };
        assert_eq!(r, logical(100.0, 100.0, 250.0, 125.0));
    }

    #[test]
    fn ctrl_edge_drag_moves_the_companion_edge() {
        let m = metrics();
        let start = logical(100.0, 100.0, 200.0, 100.0); // aspect 2.0
        let drag = begin(Some(start), LogicalPoint::from_raw(100.0, 150.0), &m);
        assert!(matches!(
            drag,
            Drag::Resizing {
                handle: Handle::Left,
                ..
            }
        ));
        // Left edge to x=50: bottom = top + (right - px)/aspect = 100 + 250/2.
        let step = advance(drag, Some(start), &input((50.0, 150.0), false, true, &m));
        let DragStep::Changed { rect: r, .. } = step else {
            panic!("resized");
        };
        assert_eq!(r, logical(50.0, 100.0, 250.0, 125.0));
    }

    #[test]
    fn crossing_through_flips_the_active_handle() {
        let m = metrics();
        let start = logical(100.0, 100.0, 100.0, 100.0);
        let drag = begin(Some(start), LogicalPoint::from_raw(200.0, 200.0), &m);
        // Drag BR far past the left edge: raw right < left -> handle flips
        // to BottomLeft and the rect normalizes (cursor side keeps tracking).
        let step = advance(drag, Some(start), &input((20.0, 250.0), false, false, &m));
        let DragStep::Changed { drag: d, rect: r } = step else {
            panic!("resized");
        };
        assert!(matches!(
            d,
            Drag::Resizing {
                handle: Handle::BottomLeft,
                ..
            }
        ));
        assert_eq!(r, logical(20.0, 100.0, 80.0, 150.0));
    }

    #[test]
    fn release_outside_an_uncommitted_click_clears_the_selection() {
        let m = metrics();
        let current = logical(100.0, 100.0, 100.0, 100.0);
        // Press far outside, release without exceeding the threshold.
        let drag = begin(Some(current), LogicalPoint::from_raw(500.0, 500.0), &m);
        assert_eq!(
            release(
                drag,
                Some(current),
                LogicalPoint::from_raw(500.0, 500.0),
                &m
            ),
            None
        );
        // The same click landing inside keeps the selection.
        let drag = begin(Some(current), LogicalPoint::from_raw(150.0, 150.0), &m);
        assert_eq!(
            release(
                drag,
                Some(current),
                LogicalPoint::from_raw(150.0, 150.0),
                &m
            ),
            Some(current)
        );
    }

    #[test]
    fn degenerate_cursor_position_stays_finite_via_min_enforcement() {
        let m = metrics();
        let start = logical(100.0, 100.0, 100.0, 100.0);
        let drag = begin(Some(start), LogicalPoint::from_raw(100.0, 100.0), &m);
        // TL drag with the cursor exactly on the opposite (BR) corner:
        // the aspect comparison is 0/0 = NaN (false), the naive edges win,
        // and the zero-size result expands to the minimum anchored at the
        // fixed BR corner.
        let step = advance(drag, Some(start), &input((200.0, 200.0), false, true, &m));
        let DragStep::Changed { rect: r, .. } = step else {
            panic!("resized");
        };
        assert!(r.width.0.is_finite() && r.height.0.is_finite());
        assert_eq!(r, logical(190.0, 190.0, 10.0, 10.0));
    }
}
