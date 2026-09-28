//! The geometry HUD: `WxH+X+Y` readout with position and hide-time
//! (draft F27).
//!
//! Flameshot parity (`capturewidget.cpp` `showxywh`/`xywhTick`/`paintEvent`):
//! every selection geometry change shows the HUD and restarts the hide
//! timer; `hud_hide_time = 0` keeps it until the next change. The position
//! config is the Flameshot `showSelectionGeometry` bounded int 0-5
//! (`generalconf.h` `xywh_position`): 0 = never, 1 = top-left,
//! 2 = bottom-left, 3 = top-right, 4 = bottom-right (the default),
//! 5 = center; unknown values fall back to center (Flameshot's switch
//! default).
//!
//! SPACE DECISION (documented deviation): Flameshot multiplies the values by
//! the device-pixel ratio because its selection lives in one screen's
//! logical space. A `FlowShot` selection is a single rect in GLOBAL LOGICAL
//! space spanning mixed-DPI outputs (#4894 restoration), where "physical"
//! has no single answer - so the HUD reports logical px, the same space the
//! layout algebra and `hyprctl monitors -j` geometry math speak.

use std::time::{Duration, Instant};

use flowshot_core::geometry::{Logical, LogicalPoint, LogicalRect};
use flowshot_core::tokens::Spacing;

use super::metrics::{AVG_GLYPH_ADVANCE, SelectionMetrics};

/// Where the HUD box sits relative to the selection (config value 0-5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HudPosition {
    /// Never shown (config 0).
    Never,
    /// Box top-left at the selection's top-left corner (config 1).
    TopLeft,
    /// Box bottom-left at the selection's bottom-left corner (config 2).
    BottomLeft,
    /// Box top-right at the selection's top-right corner (config 3).
    TopRight,
    /// Box bottom-right at the selection's bottom-right corner (config 4,
    /// the default).
    #[default]
    BottomRight,
    /// Box centered in the selection (config 5 and unknown values).
    Center,
}

impl HudPosition {
    /// Maps the `[editor].hud_position` config value (Flameshot
    /// `xywh_position` enum order).
    #[must_use]
    pub const fn from_config(value: u8) -> Self {
        match value {
            0 => Self::Never,
            1 => Self::TopLeft,
            2 => Self::BottomLeft,
            3 => Self::TopRight,
            4 => Self::BottomRight,
            _ => Self::Center,
        }
    }
}

/// The HUD's visibility window (show-on-change + hide-time deadline).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct HudTimer {
    shown_at: Option<Instant>,
}

impl HudTimer {
    /// Shows the HUD and (re)starts the hide-time countdown.
    pub(super) fn show(&mut self, now: Instant) {
        self.shown_at = Some(now);
    }

    /// Hides the HUD immediately (selection cleared).
    pub(super) fn hide(&mut self) {
        self.shown_at = None;
    }

    /// Whether the HUD is visible at `now` (`hide_time` of zero never hides,
    /// Flameshot `showxywh`: the timer only starts for a nonzero timeout).
    pub(super) fn visible(&self, now: Instant, hide_time: Duration) -> bool {
        match (self.shown_at, hide_time.is_zero()) {
            (None, _) => false,
            (Some(_), true) => true,
            (Some(shown_at), false) => now < shown_at + hide_time,
        }
    }

    /// The instant the HUD will hide, when a countdown is running (the
    /// shell's `ControlFlow::WaitUntil` wake).
    pub(super) fn wake(&self, hide_time: Duration) -> Option<Instant> {
        if hide_time.is_zero() {
            return None;
        }
        self.shown_at.map(|shown_at| shown_at + hide_time)
    }

    /// Evaluates the deadline: hides the HUD when it passed; `true` when
    /// this call hid it (the shell redraws).
    pub(super) fn tick(&mut self, now: Instant, hide_time: Duration) -> bool {
        let expired = !hide_time.is_zero()
            && self
                .shown_at
                .is_some_and(|shown_at| now >= shown_at + hide_time);
        if expired {
            self.shown_at = None;
        }
        expired
    }
}

/// The HUD content for one frame: the text and its box in global logical
/// space (paint converts per window).
#[derive(Debug, Clone, PartialEq)]
pub struct HudView {
    /// The `WxH+X+Y` readout (integer logical px, truncated toward zero -
    /// Flameshot `static_cast<int>` parity).
    pub text: String,
    /// The background box, positioned per [`HudPosition`] INSIDE the
    /// selection (Flameshot `paintEvent` switch).
    pub box_rect: LogicalRect,
    /// The text's top-left corner (the box origin plus the token padding -
    /// the same padding the box size includes).
    pub text_origin: LogicalPoint,
}

/// Builds the HUD view for `selection`, or `None` when the position config
/// is `Never`. The box size derives from the token typography (line spacing
/// plus spacing-token padding) and the estimated text advance
/// ([`AVG_GLYPH_ADVANCE`] - the shaped text itself is measured by the
/// renderer; the box is an estimate).
#[must_use]
pub(super) fn hud_view(
    selection: LogicalRect,
    position: HudPosition,
    metrics: &SelectionMetrics,
    spacing: &Spacing,
) -> Option<HudView> {
    if position == HudPosition::Never {
        return None;
    }
    let text = format_geometry(selection);
    let pad_h = f64::from(spacing.medium);
    let pad_v = f64::from(spacing.small);
    let chars = u32::try_from(text.chars().count()).unwrap_or(u32::MAX);
    let width = f64::from(chars) * metrics.font_size * AVG_GLYPH_ADVANCE + pad_h * 2.0;
    let height = metrics.font_line_spacing + pad_v * 2.0;
    let box_rect = anchor_box(position, selection, width, height);
    let text_origin =
        LogicalPoint::new(Logical(box_rect.x.0 + pad_h), Logical(box_rect.y.0 + pad_v));
    Some(HudView {
        text,
        box_rect,
        text_origin,
    })
}

/// Positions the HUD box inside the selection (Flameshot `paintEvent`
/// switch: corners anchor the corresponding box corner, center centers).
fn anchor_box(
    position: HudPosition,
    selection: LogicalRect,
    width: f64,
    height: f64,
) -> LogicalRect {
    let (x, y) = match position {
        HudPosition::TopLeft => (selection.x, selection.y),
        HudPosition::BottomLeft => (selection.x, Logical(selection.bottom().0 - height)),
        HudPosition::TopRight => (Logical(selection.right().0 - width), selection.y),
        HudPosition::BottomRight | HudPosition::Never => (
            Logical(selection.right().0 - width),
            Logical(selection.bottom().0 - height),
        ),
        HudPosition::Center => (
            Logical(selection.x.0 + (selection.width.0 - width) / 2.0),
            Logical(selection.y.0 + (selection.height.0 - height) / 2.0),
        ),
    };
    LogicalRect::new(x, y, Logical(width), Logical(height))
}

/// The `WxH+X+Y` readout in integer logical px (truncated toward zero,
/// Flameshot `QString::number(static_cast<int>(...))` parity).
#[must_use]
pub fn format_geometry(selection: LogicalRect) -> String {
    format!(
        "{}x{}+{}+{}",
        display_px(selection.width.0),
        display_px(selection.height.0),
        display_px(selection.x.0),
        display_px(selection.y.0),
    )
}

/// f64 -> integer display value: truncate toward zero (saturating at the
/// `i64` bounds; selection coordinates are layout-scale values far inside).
fn display_px(value: f64) -> i64 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "truncation toward zero IS the Flameshot display semantics (static_cast<int>); the f64 is a layout-scale coordinate"
    )]
    let truncated = value.trunc() as i64;
    truncated
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::float_cmp,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic
    )]

    use flowshot_core::tokens::DesignTokens;

    use super::*;

    fn metrics() -> SelectionMetrics {
        SelectionMetrics::from_tokens(&DesignTokens::default())
    }

    fn spacing() -> Spacing {
        DesignTokens::default().spacing
    }

    fn selection() -> LogicalRect {
        LogicalRect::from_raw(100.0, 50.0, 300.0, 200.0)
    }

    #[test]
    fn position_maps_the_flameshot_config_enum() {
        assert_eq!(HudPosition::from_config(0), HudPosition::Never);
        assert_eq!(HudPosition::from_config(1), HudPosition::TopLeft);
        assert_eq!(HudPosition::from_config(2), HudPosition::BottomLeft);
        assert_eq!(HudPosition::from_config(3), HudPosition::TopRight);
        assert_eq!(HudPosition::from_config(4), HudPosition::BottomRight);
        assert_eq!(HudPosition::from_config(5), HudPosition::Center);
        assert_eq!(HudPosition::from_config(9), HudPosition::Center);
        assert_eq!(HudPosition::default(), HudPosition::BottomRight);
    }

    #[test]
    fn text_is_wxh_plus_x_plus_y_truncated() {
        assert_eq!(format_geometry(selection()), "300x200+100+50");
        // Truncation toward zero, negative origins keep their sign.
        assert_eq!(
            format_geometry(LogicalRect::from_raw(-20.75, 0.25, 10.9, 5.1)),
            "10x5+-20+0"
        );
    }

    #[test]
    fn box_anchors_per_position_inside_the_selection() {
        let m = metrics();
        let s = spacing();
        let sel = selection();
        let view = hud_view(sel, HudPosition::TopLeft, &m, &s).unwrap();
        assert_eq!(view.box_rect.x, sel.x);
        assert_eq!(view.box_rect.y, sel.y);
        let view = hud_view(sel, HudPosition::BottomRight, &m, &s).unwrap();
        assert_eq!(view.box_rect.right().0, sel.right().0);
        assert_eq!(view.box_rect.bottom().0, sel.bottom().0);
        let view = hud_view(sel, HudPosition::Center, &m, &s).unwrap();
        let box_center_x = view.box_rect.x.0 + view.box_rect.width.0 / 2.0;
        assert!((box_center_x - (sel.x.0 + sel.width.0 / 2.0)).abs() < 1e-9);
        assert!(hud_view(sel, HudPosition::Never, &m, &s).is_none());
    }

    #[test]
    fn box_size_derives_from_tokens() {
        let m = metrics();
        let s = spacing();
        let view = hud_view(selection(), HudPosition::Center, &m, &s).unwrap();
        // "300x200+100+50" = 14 chars.
        let expected_width = 14.0 * m.font_size * AVG_GLYPH_ADVANCE + f64::from(s.medium) * 2.0;
        let expected_height = m.font_line_spacing + f64::from(s.small) * 2.0;
        assert_eq!(view.box_rect.width.0, expected_width);
        assert_eq!(view.box_rect.height.0, expected_height);
    }

    #[test]
    fn timer_shows_on_change_and_hides_after_the_deadline() {
        let hide = Duration::from_millis(3000);
        let t0 = Instant::now();
        let mut timer = HudTimer::default();
        assert!(!timer.visible(t0, hide));
        timer.show(t0);
        assert!(timer.visible(t0, hide));
        assert!(timer.visible(t0 + Duration::from_millis(2999), hide));
        assert!(!timer.visible(t0 + Duration::from_millis(3000), hide));
        assert_eq!(timer.wake(hide), Some(t0 + hide));
        // A change restarts the countdown.
        timer.show(t0 + Duration::from_millis(2000));
        assert!(timer.visible(t0 + Duration::from_millis(4000), hide));
        // tick flips exactly once at the deadline.
        assert!(!timer.tick(t0 + Duration::from_millis(4999), hide));
        assert!(timer.tick(t0 + Duration::from_millis(5000), hide));
        assert!(!timer.tick(t0 + Duration::from_millis(5001), hide));
        assert_eq!(timer.wake(hide), None);
    }

    #[test]
    fn zero_hide_time_never_hides() {
        let t0 = Instant::now();
        let mut timer = HudTimer::default();
        timer.show(t0);
        assert!(timer.visible(t0 + Duration::from_secs(3600), Duration::ZERO));
        assert_eq!(timer.wake(Duration::ZERO), None);
        assert!(!timer.tick(t0 + Duration::from_secs(3600), Duration::ZERO));
        timer.hide();
        assert!(!timer.visible(t0, Duration::ZERO));
    }
}
