//! Keyboard selection adjustments (plan todo 16, draft F27).
//!
//! CODE values win over the README (F27): every step is 1 logical px.
//! The edge semantics are Flameshot's verbatim (`selectionwidget.cpp`
//! `moveLeft`/`resizeLeft`/`symResizeLeft` slots, default bindings
//! `Left` / `Shift+Left` / `Ctrl+Shift+Left`):
//!
//! - arrows MOVE the selection 1px (`adjusted(-1, 0, -1, 0)` etc.),
//! - Shift+arrows resize ONE edge 1px - Left/Right move the RIGHT edge,
//!   Up/Down move the BOTTOM edge (`resizeLeft = adjusted(0, 0, -1, 0)`),
//! - Ctrl+Shift+arrows resize symmetrically 1px per side
//!   (`symResizeLeft = adjusted(1, 0, -1, 0)`).
//!
//! Keyboard rects clamp to the layout bounds (Flameshot
//! `setGeometryByKeyboard` intersects with the parent rect) and enforce the
//! 10x10 minimum (BORROW-MODIFIED: Flameshot floors at 1x1).

use flowshot_core::geometry::LogicalRect;
use winit::keyboard::KeyCode;

use super::resize::fit_into_bounds;

/// One keyboard selection adjustment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Nudge {
    /// Translate 1px left.
    MoveLeft,
    /// Translate 1px right.
    MoveRight,
    /// Translate 1px up.
    MoveUp,
    /// Translate 1px down.
    MoveDown,
    /// Right edge 1px left (shrinks from the right).
    ResizeLeft,
    /// Right edge 1px right.
    ResizeRight,
    /// Bottom edge 1px up.
    ResizeUp,
    /// Bottom edge 1px down.
    ResizeDown,
    /// Both vertical edges inward 1px each.
    SymLeft,
    /// Both vertical edges outward 1px each.
    SymRight,
    /// Both horizontal edges outward 1px each.
    SymUp,
    /// Both horizontal edges inward 1px each.
    SymDown,
}

/// Maps an arrow key plus modifiers to its nudge (`None` for non-arrow keys
/// and for Ctrl-only, which Flameshot leaves unbound).
#[must_use]
pub(super) fn nudge_for(code: KeyCode, shift: bool, ctrl: bool) -> Option<Nudge> {
    use Nudge::{
        MoveDown, MoveLeft, MoveRight, MoveUp, ResizeDown, ResizeLeft, ResizeRight, ResizeUp,
        SymDown, SymLeft, SymRight, SymUp,
    };
    let plain = match code {
        KeyCode::ArrowLeft => (MoveLeft, ResizeLeft, SymLeft),
        KeyCode::ArrowRight => (MoveRight, ResizeRight, SymRight),
        KeyCode::ArrowUp => (MoveUp, ResizeUp, SymUp),
        KeyCode::ArrowDown => (MoveDown, ResizeDown, SymDown),
        _ => return None,
    };
    let (plain, resize, symmetric) = plain;
    Some(match (shift, ctrl) {
        (false, false) => plain,
        (true, false) => resize,
        (true, true) => symmetric,
        (false, true) => return None,
    })
}

/// Applies one nudge: edges adjust by 1px, the minimum holds at the fixed
/// edge (symmetric nudges hold the center), and the result intersects the
/// layout bounds. A nudge that cannot produce a valid rect (degenerate
/// bounds) leaves the selection unchanged.
#[must_use]
pub(super) fn apply_nudge(
    rect: LogicalRect,
    nudge: Nudge,
    min: f64,
    bounds: Option<LogicalRect>,
) -> LogicalRect {
    let (mut left, mut top) = (rect.x.0, rect.y.0);
    let (mut right, mut bottom) = (rect.right().0, rect.bottom().0);
    match nudge {
        Nudge::MoveLeft => {
            left -= 1.0;
            right -= 1.0;
        }
        Nudge::MoveRight => {
            left += 1.0;
            right += 1.0;
        }
        Nudge::MoveUp => {
            top -= 1.0;
            bottom -= 1.0;
        }
        Nudge::MoveDown => {
            top += 1.0;
            bottom += 1.0;
        }
        Nudge::ResizeLeft => right = (right - 1.0).max(left + min),
        Nudge::ResizeRight => right += 1.0,
        Nudge::ResizeUp => bottom = (bottom - 1.0).max(top + min),
        Nudge::ResizeDown => bottom += 1.0,
        Nudge::SymLeft => {
            left += 1.0;
            right -= 1.0;
            if right - left < min {
                // At the minimum the symmetric shrink is a no-op: re-anchor
                // around the original center.
                let center = f64::midpoint(rect.x.0, rect.right().0);
                left = center - min / 2.0;
                right = center + min / 2.0;
            }
        }
        Nudge::SymRight => {
            left -= 1.0;
            right += 1.0;
        }
        Nudge::SymUp => {
            top -= 1.0;
            bottom += 1.0;
        }
        Nudge::SymDown => {
            top += 1.0;
            bottom -= 1.0;
            if bottom - top < min {
                let center = f64::midpoint(rect.y.0, rect.bottom().0);
                top = center - min / 2.0;
                bottom = center + min / 2.0;
            }
        }
    }
    let adjusted = LogicalRect::from_raw(left, top, right - left, bottom - top);
    match nudge {
        // Moves slide (size preserved); resizes intersect (the fixed edge
        // stays put - Flameshot `setGeometryByKeyboard`).
        Nudge::MoveLeft | Nudge::MoveRight | Nudge::MoveUp | Nudge::MoveDown => {
            fit_into_bounds(adjusted, bounds)
        }
        _ => match bounds {
            Some(bounds) => bounds.intersection(&adjusted),
            None => Some(adjusted),
        }
        .unwrap_or(rect),
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::float_cmp,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::unnecessary_wraps
    )]

    use super::*;

    const MIN: f64 = 10.0;

    fn bounds() -> Option<LogicalRect> {
        Some(LogicalRect::from_raw(0.0, 0.0, 1920.0, 1080.0))
    }

    fn rect(x: f64, y: f64, w: f64, h: f64) -> LogicalRect {
        LogicalRect::from_raw(x, y, w, h)
    }

    #[test]
    fn arrows_map_per_modifier_combo() {
        use KeyCode::{ArrowDown, ArrowLeft, ArrowRight, ArrowUp, KeyA};
        assert_eq!(nudge_for(ArrowLeft, false, false), Some(Nudge::MoveLeft));
        assert_eq!(nudge_for(ArrowLeft, true, false), Some(Nudge::ResizeLeft));
        assert_eq!(nudge_for(ArrowLeft, true, true), Some(Nudge::SymLeft));
        assert_eq!(nudge_for(ArrowLeft, false, true), None);
        assert_eq!(nudge_for(ArrowRight, false, false), Some(Nudge::MoveRight));
        assert_eq!(nudge_for(ArrowRight, true, false), Some(Nudge::ResizeRight));
        assert_eq!(nudge_for(ArrowRight, true, true), Some(Nudge::SymRight));
        assert_eq!(nudge_for(ArrowUp, true, false), Some(Nudge::ResizeUp));
        assert_eq!(nudge_for(ArrowUp, true, true), Some(Nudge::SymUp));
        assert_eq!(nudge_for(ArrowDown, true, false), Some(Nudge::ResizeDown));
        assert_eq!(nudge_for(ArrowDown, true, true), Some(Nudge::SymDown));
        assert_eq!(nudge_for(KeyA, true, true), None);
    }

    #[test]
    fn move_translates_one_px() {
        let r = rect(100.0, 100.0, 50.0, 50.0);
        assert_eq!(
            apply_nudge(r, Nudge::MoveLeft, MIN, bounds()),
            rect(99.0, 100.0, 50.0, 50.0)
        );
        assert_eq!(
            apply_nudge(r, Nudge::MoveRight, MIN, bounds()),
            rect(101.0, 100.0, 50.0, 50.0)
        );
        assert_eq!(
            apply_nudge(r, Nudge::MoveUp, MIN, bounds()),
            rect(100.0, 99.0, 50.0, 50.0)
        );
        assert_eq!(
            apply_nudge(r, Nudge::MoveDown, MIN, bounds()),
            rect(100.0, 101.0, 50.0, 50.0)
        );
    }

    #[test]
    fn shift_arrows_move_right_or_bottom_edge_only() {
        let r = rect(100.0, 100.0, 50.0, 50.0);
        // Flameshot code: resizeLeft shrinks the RIGHT edge.
        assert_eq!(
            apply_nudge(r, Nudge::ResizeLeft, MIN, bounds()),
            rect(100.0, 100.0, 49.0, 50.0)
        );
        assert_eq!(
            apply_nudge(r, Nudge::ResizeRight, MIN, bounds()),
            rect(100.0, 100.0, 51.0, 50.0)
        );
        assert_eq!(
            apply_nudge(r, Nudge::ResizeUp, MIN, bounds()),
            rect(100.0, 100.0, 50.0, 49.0)
        );
        assert_eq!(
            apply_nudge(r, Nudge::ResizeDown, MIN, bounds()),
            rect(100.0, 100.0, 50.0, 51.0)
        );
    }

    #[test]
    fn ctrl_shift_arrows_resize_symmetrically() {
        let r = rect(100.0, 100.0, 50.0, 50.0);
        assert_eq!(
            apply_nudge(r, Nudge::SymLeft, MIN, bounds()),
            rect(101.0, 100.0, 48.0, 50.0)
        );
        assert_eq!(
            apply_nudge(r, Nudge::SymRight, MIN, bounds()),
            rect(99.0, 100.0, 52.0, 50.0)
        );
        assert_eq!(
            apply_nudge(r, Nudge::SymUp, MIN, bounds()),
            rect(100.0, 99.0, 50.0, 52.0)
        );
        assert_eq!(
            apply_nudge(r, Nudge::SymDown, MIN, bounds()),
            rect(100.0, 101.0, 50.0, 48.0)
        );
    }

    #[test]
    fn resize_stops_at_the_minimum() {
        let r = rect(100.0, 100.0, 10.0, 10.0);
        // Shrinking edges clamp at min; growing edges are unaffected.
        assert_eq!(apply_nudge(r, Nudge::ResizeLeft, MIN, bounds()), r);
        assert_eq!(apply_nudge(r, Nudge::ResizeUp, MIN, bounds()), r);
        assert_eq!(apply_nudge(r, Nudge::SymLeft, MIN, bounds()), r);
        assert_eq!(apply_nudge(r, Nudge::SymDown, MIN, bounds()), r);
        assert_eq!(
            apply_nudge(r, Nudge::ResizeRight, MIN, bounds()),
            rect(100.0, 100.0, 11.0, 10.0)
        );
    }

    #[test]
    fn move_slides_at_the_bounds_edge() {
        let r = rect(0.0, 0.0, 50.0, 50.0);
        // Already at the left bound: the move cannot slide further.
        assert_eq!(apply_nudge(r, Nudge::MoveLeft, MIN, bounds()), r);
        let corner = rect(1870.0, 1030.0, 50.0, 50.0);
        assert_eq!(
            apply_nudge(corner, Nudge::MoveRight, MIN, bounds()),
            rect(1870.0, 1030.0, 50.0, 50.0)
        );
    }

    #[test]
    fn resize_intersects_the_bounds() {
        // Right edge at the bound: ResizeRight intersects back (fixed edge
        // stays put, growth is refused).
        let r = rect(1820.0, 100.0, 100.0, 50.0);
        assert_eq!(apply_nudge(r, Nudge::ResizeRight, MIN, bounds()), r);
        // Growing inward from the bound works.
        assert_eq!(
            apply_nudge(r, Nudge::ResizeLeft, MIN, bounds()),
            rect(1820.0, 100.0, 99.0, 50.0)
        );
    }
}
