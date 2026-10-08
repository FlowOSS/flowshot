//! Tooltip widget + the 400ms show-delay clock (Flameshot parity constant).
//!
//! [`TooltipClock`] is pure state: the chrome motion owner feeds it hover
//! retargets and reads `ready`/`deadline` at the frame's `now`, so the
//! delayed show participates in the shell's wake scheduling (a settled
//! overlay still wakes exactly once at the deadline - the idle zero-CPU
//! contract holds).

use std::time::{Duration, Instant};

use crate::render::{
    Color, DisplayList, Rect, ShadowSpec, Shape, TextAnchor, TextCommand, f32_from_f64,
};
use crate::selection::AVG_GLYPH_ADVANCE;
use flowshot_core::tokens::DesignTokens;

/// The hover-to-show delay (Flameshot: Qt's tooltip delay, 400ms).
pub const TOOLTIP_DELAY: Duration = Duration::from_millis(400);

/// Tooltip background opacity (0-255): near-solid contrast ink (the HUD's
/// 200 reads too translucent for small text over arbitrary wallpaper).
const TOOLTIP_BG_ALPHA: u8 = 235;

/// Text line height ratio (the render stack's standard).
const LINE_HEIGHT_RATIO: f32 = 1.2;

/// The tooltip show-delay state: which button is hovered and since when.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TooltipClock {
    hover: Option<usize>,
    since: Option<Instant>,
}

impl TooltipClock {
    /// Retargets to the hovered element (`None` = off every element); the
    /// delay restarts whenever the target changes.
    pub fn retarget(&mut self, index: Option<usize>, now: Instant) {
        if self.hover == index {
            return;
        }
        self.hover = index;
        self.since = index.map(|_| now);
    }

    /// Clears the clock (a press consumed the hover: the tooltip stays away
    /// until the pointer re-enters - Qt parity).
    pub fn reset(&mut self) {
        self.hover = None;
        self.since = None;
    }

    /// The element whose tooltip shows at `now`, when the delay has run.
    #[must_use]
    pub fn ready(&self, now: Instant) -> Option<usize> {
        let since = self.since?;
        if now >= since + TOOLTIP_DELAY {
            self.hover
        } else {
            None
        }
    }

    /// The pending show deadline (the wake-scheduling seam): `Some` only
    /// while a tooltip is armed and its delay has NOT yet run.
    #[must_use]
    pub fn deadline(&self, now: Instant) -> Option<Instant> {
        let deadline = self.since? + TOOLTIP_DELAY;
        (self.hover.is_some() && deadline > now).then_some(deadline)
    }
}

/// A tooltip: a small dark pill with one text line, anchored to a widget.
#[derive(Debug, Clone, PartialEq)]
pub struct Tooltip {
    /// The bounding rectangle (physical px).
    pub rect: Rect,
    /// The tooltip text.
    pub text: String,
}

impl Tooltip {
    /// Measures the tooltip box for `text` (the width is the HUD's
    /// estimated-advance model: the shaped text itself is renderer-measured,
    /// the box is an estimate, documented as such).
    #[must_use]
    pub fn measured_size(text: &str, tokens: &DesignTokens, scale: f32) -> (f32, f32) {
        let font_size = tokens.typography.base_size as f32 * scale;
        let pad_h = tokens.spacing.medium as f32 * scale;
        let pad_v = tokens.spacing.small as f32 * scale;
        let chars = text.chars().count() as f32;
        let width = chars * font_size * f32_from_f64(AVG_GLYPH_ADVANCE) + pad_h * 2.0;
        (width, font_size * LINE_HEIGHT_RATIO + pad_v * 2.0)
    }

    /// Anchors a tooltip above `anchor`, centered on it: flips below near
    /// the top edge and clamps horizontally into `bounds` (the popover's
    /// flip-at-edges rule).
    #[must_use]
    pub fn anchored(
        anchor: Rect,
        text: &str,
        tokens: &DesignTokens,
        scale: f32,
        bounds: Rect,
    ) -> Self {
        let (width, height) = Self::measured_size(text, tokens, scale);
        let gap = tokens.spacing.small as f32 * scale;
        let margin = tokens.spacing.small as f32 * scale;
        let x = (anchor.center().x - width / 2.0)
            .clamp(margin, (bounds.right() - width - margin).max(margin));
        let mut y = anchor.origin.y - gap - height;
        if y < margin {
            y = (anchor.bottom() + gap).min((bounds.bottom() - height - margin).max(margin));
        }
        Self {
            rect: Rect::from_parts(x, y, width, height),
            text: text.to_owned(),
        }
    }

    /// Draws the tooltip into the display list.
    pub fn draw(&self, list: &mut DisplayList, tokens: &DesignTokens, scale: f32) {
        let contrast = Color::from_hex_token(&tokens.palette.contrast)
            .unwrap_or(Color::from_rgba8(255, 0, 255, 255));
        let background = contrast.with_alpha8(TOOLTIP_BG_ALPHA);
        let radius = tokens.radii.medium as f32 * scale;

        if let Some(spec) = ShadowSpec::from_token(&tokens.shadows.small, scale) {
            list.shadow(self.rect, radius, spec);
        }
        list.fill(
            Shape::Rect {
                rect: self.rect,
                radius,
            },
            background,
        );

        let font_size = tokens.typography.base_size as f32 * scale;
        let line_height = font_size * LINE_HEIGHT_RATIO;
        list.text(TextCommand {
            position: crate::render::Point::new(
                self.rect.origin.x + tokens.spacing.medium as f32 * scale,
                self.rect.origin.y + (self.rect.size.height - line_height) / 2.0,
            ),
            text: self.text.clone(),
            font_size,
            line_height,
            color: background.readable_ink(),
            family: Some(tokens.typography.family.clone()),
            max_width: None,
            anchor: TextAnchor::TopLeft,
            bold: false,
        });
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::float_cmp
    )]

    use super::*;

    fn tokens() -> DesignTokens {
        DesignTokens::default()
    }

    #[test]
    fn clock_stays_dark_before_the_delay_and_ready_after() {
        let t0 = Instant::now();
        let mut clock = TooltipClock::default();
        assert_eq!(clock.ready(t0), None);
        clock.retarget(Some(3), t0);
        assert_eq!(clock.ready(t0), None);
        assert_eq!(clock.ready(t0 + Duration::from_millis(399)), None);
        assert_eq!(clock.ready(t0 + TOOLTIP_DELAY), Some(3));
        assert_eq!(
            clock.deadline(t0 + Duration::from_millis(399)),
            Some(t0 + TOOLTIP_DELAY)
        );
        assert_eq!(clock.deadline(t0 + TOOLTIP_DELAY), None, "shown: no wake");
    }

    #[test]
    fn clock_restarts_the_delay_on_every_target_change() {
        let t0 = Instant::now();
        let mut clock = TooltipClock::default();
        clock.retarget(Some(0), t0);
        let switch = t0 + Duration::from_millis(300);
        clock.retarget(Some(1), switch);
        assert_eq!(clock.ready(switch + Duration::from_millis(300)), None);
        assert_eq!(clock.ready(switch + TOOLTIP_DELAY), Some(1));
        clock.retarget(None, switch);
        assert_eq!(clock.ready(switch + Duration::from_secs(1)), None);
        assert_eq!(clock.deadline(switch), None);
    }

    #[test]
    fn reset_suppresses_until_the_target_changes() {
        let t0 = Instant::now();
        let mut clock = TooltipClock::default();
        clock.retarget(Some(2), t0);
        clock.reset();
        assert_eq!(clock.ready(t0 + Duration::from_secs(1)), None);
        assert_eq!(clock.deadline(t0), None);
        // Re-entry (None -> Some) re-arms with a fresh delay.
        clock.retarget(None, t0);
        clock.retarget(Some(2), t0);
        assert_eq!(clock.ready(t0 + Duration::from_secs(1)), Some(2));
    }

    #[test]
    fn anchored_flips_below_at_the_top_edge_and_clamps_horizontally() {
        let t = tokens();
        let bounds = Rect::from_parts(0.0, 0.0, 1920.0, 1080.0);
        let anchor = Rect::from_parts(900.0, 500.0, 28.0, 28.0);
        let tip = Tooltip::anchored(anchor, "Pencil — freehand draw [P]", &t, 1.0, bounds);
        assert!(tip.rect.bottom() <= anchor.origin.y, "above the anchor");
        assert!((tip.rect.center().x - anchor.center().x).abs() < 1e-3);

        let top = Rect::from_parts(900.0, 10.0, 28.0, 28.0);
        let flipped = Tooltip::anchored(top, "Text [T]", &t, 1.0, bounds);
        assert!(flipped.rect.origin.y >= top.bottom(), "flipped below");

        let edge = Rect::from_parts(5.0, 500.0, 28.0, 28.0);
        let clamped = Tooltip::anchored(edge, "Rectangle — rectangle outline [R]", &t, 1.0, bounds);
        assert!(clamped.rect.origin.x >= 0.0);
        assert!(clamped.rect.right() <= bounds.right());
    }

    #[test]
    fn draw_emits_shadow_fill_and_text() {
        let t = tokens();
        let mut list = DisplayList::new();
        let tip = Tooltip::anchored(
            Rect::from_parts(100.0, 100.0, 28.0, 28.0),
            "Grid [F]",
            &t,
            1.0,
            Rect::from_parts(0.0, 0.0, 1920.0, 1080.0),
        );
        tip.draw(&mut list, &t, 1.0);
        let commands: Vec<_> = list.iter().collect();
        assert!(commands.len() >= 2);
        assert!(matches!(
            commands.last().unwrap(),
            crate::render::Command::Text(text) if text.text == "Grid [F]"
        ));
    }
}
