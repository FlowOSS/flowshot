//! Zoom-to-cursor and size-clamp math (pure functions, plan todo 30).
//!
//! # The anchor invariant (F27 "zoom-from-center AVOID")
//!
//! Zooming must keep the image point under the cursor under the cursor. In
//! window-local physical px, with the image drawn at `offset` inside the
//! content area (window minus [`super::spec::MARGIN`] frame) at zoom factor
//! `scale`, the image point under the cursor is
//! `p = (cursor - margin - offset) / scale`. After the zoom commits
//! (`scale'`) the window resizes and the compositor re-places it; with the
//! window's new top-left shifted by `delta` on screen, the offset that
//! restores the invariant is
//! `offset' = cursor - margin - p * scale' - delta`.
//!
//! `delta` is compositor policy and cannot be observed on Wayland (no
//! position events for toplevels). Hyprland 0.56.2 was probed live
//! (2026-09-26): floating-window resizes keep the window CENTER invariant,
//! so [`ResizeAnchor::Center`] (`delta = (old - new) / 2`) is the default.
//! [`ResizeAnchor::TopLeft`] covers compositors that pin the top-left
//! corner. Near monitor edges Hyprland may additionally shift the window
//! back on-screen; the anchor then drifts by that shift (documented
//! limitation, no Wayland API exists to compensate).
//!
//! # Resize mechanics on Hyprland (live-probed 2026-09-26)
//!
//! `Window::request_inner_size` is a NO-OP: Hyprland sends all four
//! `XDG_TOPLEVEL_STATE_TILED_*` states on every toplevel (XDGShell.cpp), so
//! winit classifies even floating windows as compositor-sized ("stateful")
//! and refuses client resizes. The working path is compositor-driven:
//! pinning `set_min_inner_size` + `set_max_inner_size` to the target makes
//! Hyprland reconfigure the floating window to exactly that size (growth
//! AND shrink verified), and winit reports the real extent via
//! `WindowEvent::Resized`.

use super::spec::{WHEEL_UNITS_PER_STEP, ZOOM_STEP};

/// Where the compositor keeps a floating window anchored across a
/// client-driven resize (see the module header for the live probe).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResizeAnchor {
    /// The window center stays at the same screen point (Hyprland 0.56).
    #[default]
    Center,
    /// The window top-left corner stays at the same screen point.
    TopLeft,
}

impl ResizeAnchor {
    /// The top-left screen shift the compositor applies when a window of
    /// `old` extent becomes `new` (per axis, physical px).
    #[must_use]
    pub fn delta(self, old: f64, new: f64) -> f64 {
        match self {
            Self::Center => (old - new) / 2.0,
            Self::TopLeft => 0.0,
        }
    }
}

/// Zoom-factor clamp bounds derived from the screen fit and the `MIN_SIZE`
/// floor (F27 `qBound(MIN_SIZE, ..., maximum)` parity, uniform-scale:
/// Flameshot's per-axis `qBound` with `KeepAspectRatio` can distort aspect
/// ratios - `FlowShot` clamps the single scale factor instead).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScaleBounds {
    /// Smallest allowed scale (both content axes >= `min_size`).
    pub min: f64,
    /// Largest allowed scale (window fits the screen).
    pub max: f64,
}

/// Derives the scale bounds for an `image` (physical px) shown in a window
/// with `margin` frame on a `screen` (physical px), honoring the
/// `min_size` content floor.
///
/// Degenerate screens (smaller than the frame) and `min > max` conflicts
/// resolve toward `max`: fitting the screen wins over the `MIN_SIZE` floor
/// (a pin can never be larger than the screen - plan todo 30 failure QA).
#[must_use]
pub fn scale_bounds(
    image: (u32, u32),
    screen: (u32, u32),
    margin: f64,
    min_size: f64,
) -> ScaleBounds {
    let (img_w, img_h) = (f64::from(image.0), f64::from(image.1));
    let (scr_w, scr_h) = (f64::from(screen.0), f64::from(screen.1));
    let frame = 2.0 * margin.max(0.0);
    let fit = |available: f64, img: f64| {
        if img > 0.0 {
            (available - frame).max(1.0) / img
        } else {
            1.0
        }
    };
    let floor = |img: f64| {
        if img > 0.0 {
            min_size.max(1.0) / img
        } else {
            1.0
        }
    };
    let max = fit(scr_w, img_w).min(fit(scr_h, img_h));
    let min = floor(img_w).max(floor(img_h));
    ScaleBounds { min, max }
}

/// Clamps `scale` into `bounds`; when the bounds conflict (screen smaller
/// than the `MIN_SIZE` floor) the screen-fit maximum wins.
#[must_use]
pub fn clamp_scale(scale: f64, bounds: ScaleBounds) -> f64 {
    if !scale.is_finite() {
        return bounds.max;
    }
    if bounds.min > bounds.max {
        scale.min(bounds.max)
    } else {
        scale.clamp(bounds.min, bounds.max)
    }
}

/// The zoom factor after `steps` committed wheel/pinch steps (signed;
/// multiplicative so `+n` then `-n` round-trips exactly - see
/// [`super::spec::ZOOM_STEP`]).
#[must_use]
pub fn zoom_stepped(scale: f64, steps: i32) -> f64 {
    let factor = 1.0 + ZOOM_STEP;
    let mut result = scale;
    for _ in 0..steps.abs() {
        result = if steps > 0 {
            result * factor
        } else {
            result / factor
        };
    }
    result
}

/// The image display size in physical px at `scale` (rounded).
#[must_use]
pub fn content_size(image: (u32, u32), scale: f64) -> (u32, u32) {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "rounded and clamped into [1, u32::MAX] before the cast"
    )]
    let dim = |d: u32| {
        (f64::from(d) * scale)
            .round()
            .clamp(1.0, f64::from(u32::MAX)) as u32
    };
    (dim(image.0), dim(image.1))
}

/// The window size for `content` plus the shadow frame (physical px).
#[must_use]
pub fn window_size(content: (u32, u32), margin: f64) -> (u32, u32) {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the margin token is a small positive constant; rounded before the cast"
    )]
    let frame = (2.0 * margin.max(0.0)).round() as u32;
    (
        content.0.saturating_add(frame),
        content.1.saturating_add(frame),
    )
}

/// Everything one committed zoom needs to re-anchor the image (grouped
/// value object: the eight quantities are inherent to the derivation in
/// the module header). All lengths are window-local physical px.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZoomChange {
    /// Cursor position anchoring the zoom.
    pub cursor: (f64, f64),
    /// Shadow frame width.
    pub margin: f64,
    /// Image offset before the zoom.
    pub offset: (f64, f64),
    /// Zoom factor before.
    pub scale: f64,
    /// Zoom factor after.
    pub new_scale: f64,
    /// Window extent before.
    pub old_window: (f64, f64),
    /// Window extent after.
    pub new_window: (f64, f64),
    /// The compositor's resize-anchor policy.
    pub anchor: ResizeAnchor,
}

/// The image offset that keeps the cursor-anchored image point fixed across
/// a zoom commit (see the module-header derivation).
#[must_use]
pub fn anchored_offset(change: &ZoomChange) -> (f64, f64) {
    let axis = |cursor: f64, offset: f64, old: f64, new: f64| {
        let p = if change.scale == 0.0 {
            0.0
        } else {
            (cursor - change.margin - offset) / change.scale
        };
        cursor - change.margin - p * change.new_scale - change.anchor.delta(old, new)
    };
    (
        axis(
            change.cursor.0,
            change.offset.0,
            change.old_window.0,
            change.new_window.0,
        ),
        axis(
            change.cursor.1,
            change.offset.1,
            change.old_window.1,
            change.new_window.1,
        ),
    )
}

/// Feeds `units` into the wheel accumulator and returns the number of
/// committed zoom steps (signed). One discrete notch (120 units) commits
/// one step; high-resolution deltas accumulate until they cross notch
/// boundaries (F27 accumulate-then-commit; the remainder is retained).
#[must_use]
pub fn commit_wheel(accumulator: &mut f64, units: f64) -> i32 {
    if !units.is_finite() {
        return 0;
    }
    *accumulator += units;
    let steps = (*accumulator / WHEEL_UNITS_PER_STEP).trunc();
    #[expect(
        clippy::cast_possible_truncation,
        reason = "truncated and clamped into the i32 range before the cast"
    )]
    let steps_i32 = steps.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
    *accumulator -= steps * WHEEL_UNITS_PER_STEP;
    steps_i32
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::*;

    #[test]
    fn bounds_fit_screen_and_honor_min_size() {
        // 400x300 image, 1920x1080 screen, margin 7, floor 100.
        let bounds = scale_bounds((400, 300), (1920, 1080), 7.0, 100.0);
        // max: min((1920-14)/400, (1080-14)/300) = min(4.765, 3.553)
        assert!((bounds.max - (1066.0 / 300.0)).abs() < 1e-9);
        // min: max(100/400, 100/300) = 1/3
        assert!((bounds.min - 1.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn oversized_image_clamps_to_screen() {
        // 3000x2000 on 1920x1080: height-limited.
        let bounds = scale_bounds((3000, 2000), (1920, 1080), 7.0, 100.0);
        assert!((bounds.max - (1066.0 / 2000.0)).abs() < 1e-9);
        let clamped = clamp_scale(1.0, bounds);
        let content = content_size((3000, 2000), clamped);
        let window = window_size(content, 7.0);
        assert!(window.0 <= 1920 && window.1 <= 1080);
        assert_eq!(window.1, 1080);
    }

    #[test]
    fn tiny_image_starts_magnified_to_the_floor() {
        let bounds = scale_bounds((40, 20), (1920, 1080), 7.0, 100.0);
        // min: max(100/40, 100/20) = 5
        assert!((bounds.min - 5.0).abs() < 1e-9);
        let clamped = clamp_scale(1.0, bounds);
        assert!((clamped - 5.0).abs() < 1e-9);
        assert_eq!(content_size((40, 20), clamped), (200, 100));
    }

    #[test]
    fn screen_smaller_than_floor_prefers_screen_fit() {
        let bounds = scale_bounds((400, 300), (120, 110), 7.0, 100.0);
        assert!(bounds.min > bounds.max);
        let clamped = clamp_scale(1.0, bounds);
        assert!((clamped - bounds.max).abs() < 1e-9);
        let window = window_size(content_size((400, 300), clamped), 7.0);
        assert!(window.0 <= 120 && window.1 <= 110);
    }

    #[test]
    fn zoom_steps_are_multiplicative_and_reversible() {
        let up = zoom_stepped(1.0, 5);
        assert!((up - 1.03f64.powi(5)).abs() < 1e-12);
        let round_trip = zoom_stepped(up, -5);
        assert!((round_trip - 1.0).abs() < 1e-12);
        assert!((zoom_stepped(2.0, 0) - 2.0).abs() < 1e-12);
    }

    #[test]
    fn wheel_accumulator_commits_per_notch_and_retains_remainder() {
        let mut acc = 0.0;
        assert_eq!(commit_wheel(&mut acc, 120.0), 1);
        assert_eq!(commit_wheel(&mut acc, -120.0), -1);
        // High-resolution deltas accumulate.
        assert_eq!(commit_wheel(&mut acc, 40.0), 0);
        assert_eq!(commit_wheel(&mut acc, 40.0), 0);
        assert_eq!(commit_wheel(&mut acc, 40.0), 1);
        assert!((acc - 0.0).abs() < 1e-9);
        // A single large delta commits multiple steps.
        assert_eq!(commit_wheel(&mut acc, 300.0), 2);
        assert!((acc - 60.0).abs() < 1e-9);
        assert_eq!(commit_wheel(&mut acc, f64::NAN), 0);
    }

    #[test]
    fn anchor_keeps_cursor_point_fixed_center_policy() {
        // 414x314 window (400x300 content + 2*7), cursor at (107, 82) =
        // image point p = (100, 75) at scale 1. Zoom x1.03: content
        // 412x309, window 426x323.
        let new_scale = 1.03;
        let new_window = (426.0, 323.0);
        let offset = anchored_offset(&ZoomChange {
            cursor: (107.0, 82.0),
            margin: 7.0,
            offset: (0.0, 0.0),
            scale: 1.0,
            new_scale,
            old_window: (414.0, 314.0),
            new_window,
            anchor: ResizeAnchor::Center,
        });
        // The image point under the cursor: screen-stable by construction.
        // Window-local check: margin + offset + p*new_scale + delta must
        // equal the cursor.
        let delta_x = (414.0 - new_window.0) / 2.0;
        let delta_y = (314.0 - new_window.1) / 2.0;
        let x = 7.0 + offset.0 + 100.0 * new_scale + delta_x;
        let y = 7.0 + offset.1 + 75.0 * new_scale + delta_y;
        assert!((x - 107.0).abs() < 1e-9);
        assert!((y - 82.0).abs() < 1e-9);
    }

    #[test]
    fn anchor_top_left_policy_has_zero_delta() {
        let offset = anchored_offset(&ZoomChange {
            cursor: (50.0, 50.0),
            margin: 7.0,
            offset: (0.0, 0.0),
            scale: 1.0,
            new_scale: 2.0,
            old_window: (100.0, 100.0),
            new_window: (200.0, 200.0),
            anchor: ResizeAnchor::TopLeft,
        });
        // p = 43; offset' = 50 - 7 - 86 = -43
        assert!((offset.0 - (-43.0)).abs() < 1e-9);
        assert!((offset.1 - (-43.0)).abs() < 1e-9);
    }

    #[test]
    fn center_zoom_at_window_center_needs_no_offset_beyond_margin() {
        // Cursor exactly at the content center of a symmetric resize: the
        // center anchor keeps the content centered (offset stays ~0).
        let offset = anchored_offset(&ZoomChange {
            cursor: (207.0, 157.0),
            margin: 7.0,
            offset: (0.0, 0.0),
            scale: 1.0,
            new_scale: 1.03,
            old_window: (414.0, 314.0),
            new_window: (426.0, 323.0),
            anchor: ResizeAnchor::Center,
        });
        // p = 200,150; delta = -6,-4.5; offset' = 207-7-206+6 = 0
        assert!(offset.0.abs() < 1e-9);
        assert!(offset.1.abs() < 1e-9);
    }
}
