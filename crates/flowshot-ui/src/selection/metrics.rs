//! Token-derived selection metrics.
//!
//! Every size the selection engine needs derives from the design tokens via
//! the Flameshot formulas - zero hardcoded visual constants:
//!
//! - `font_line_spacing = typography.base_size * ` [`LINE_SPACING_RATIO`]
//! - `button_base_size = font_line_spacing * ` [`BUTTON_BASE_FACTOR`]
//!   (Flameshot: `globalvalues.cpp:14`, `buttonBaseSize = fontLineSpacing * 2.2`)
//! - `handle_area = button_base_size * ` [`HANDLE_AREA_FACTOR`] and
//!   `grip = handle_area / ` [`GRIP_DIVISOR`] (Flameshot: `selectionwidget.cpp`
//!   constructor - `sideVal = buttonBaseSize() * 0.6`, `handleSide = sideVal / 2`)
//!
//! The behavior constants (drag threshold, minimum size) are spec values,
//! not theme values: they are NOT tunable beyond the config keys the
//! selection engine consumes - a deliberate no-tuning rule.

use flowshot_core::tokens::DesignTokens;

/// Line-spacing factor applied to the typography base size to obtain the
/// font line spacing (the render stack's standard ratio; `cosmic-text`
/// metrics need an explicit line height, see the `render_smoke` precedent).
pub const LINE_SPACING_RATIO: f64 = 1.2;

/// `buttonBaseSize = fontLineSpacing * 2.2` (Flameshot, `globalvalues.cpp:14`).
pub const BUTTON_BASE_FACTOR: f64 = 2.2;

/// Handle hit-area side = `buttonBaseSize * 0.6` (Flameshot, `selectionwidget.cpp`
/// constructor).
pub const HANDLE_AREA_FACTOR: f64 = 0.6;

/// Visual grip side = `area / 2` (Flameshot, `selectionwidget.cpp` constructor).
pub const GRIP_DIVISOR: f64 = 2.0;

/// Minimum selection side in logical px. BORROW-MODIFIED (Flameshot): Flameshot
/// enforces 1x1; `FlowShot` enforces 10x10 so a selection is always a usable
/// capture region.
pub const MIN_SELECTION_SIDE: f64 = 10.0;

/// Drag-create threshold in logical px, manhattan distance, strictly greater
/// (Flameshot: `capturewidget.cpp` L46 `MOUSE_DISTANCE_TO_START_MOVING 3`, L980
/// `manhattanLength() > 3`) - a click is never a selection.
pub const DRAG_THRESHOLD: f64 = 3.0;

/// Selection outline width in logical px (Flameshot: `selectionwidget.cpp`
/// `paintEvent` draws with the default 1px cosmetic pen).
pub const OUTLINE_WIDTH: f64 = 1.0;

/// HUD background opacity, 0-255 (Flameshot: `capturewidget.cpp` `paintEvent`
/// fills the xywh box with `uicolor` at alpha 200).
pub const HUD_BACKGROUND_ALPHA: u8 = 200;

/// Average glyph advance as a fraction of the font size, used ONLY to size
/// the HUD background box (the shaped text itself is measured by the
/// renderer; the box is an estimate, documented as such).
pub const AVG_GLYPH_ADVANCE: f64 = 0.6;

/// Double-click interval (Qt's default `doubleClickInterval`, which
/// Flameshot inherits: its double-click copy fires from Qt's synthesized
/// `mouseDoubleClickEvent`).
pub const DOUBLE_CLICK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(400);

/// The selection engine's derived sizes, all in logical px.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SelectionMetrics {
    /// The HUD/typography font size (`typography.base_size`).
    pub font_size: f64,
    /// Font line spacing (`typography.base_size * ` [`LINE_SPACING_RATIO`]).
    pub font_line_spacing: f64,
    /// `font_line_spacing * ` [`BUTTON_BASE_FACTOR`] (Flameshot `buttonBaseSize`).
    pub button_base_size: f64,
    /// Side of a handle's square hit area (`button_base_size * `
    /// [`HANDLE_AREA_FACTOR`]).
    pub handle_area: f64,
    /// Side of a handle's visual grip (`handle_area / ` [`GRIP_DIVISOR`]).
    pub grip: f64,
    /// Minimum selection side ([`MIN_SELECTION_SIDE`], BORROW-MODIFIED).
    pub min_side: f64,
    /// Drag-create manhattan threshold ([`DRAG_THRESHOLD`]).
    pub drag_threshold: f64,
}

impl SelectionMetrics {
    /// Derives every metric from the design tokens (the only input - the
    /// formulas are the Flameshot spec constants above).
    #[must_use]
    pub fn from_tokens(tokens: &DesignTokens) -> Self {
        let font_size = f64::from(tokens.typography.base_size);
        let font_line_spacing = font_size * LINE_SPACING_RATIO;
        let button_base_size = font_line_spacing * BUTTON_BASE_FACTOR;
        let handle_area = button_base_size * HANDLE_AREA_FACTOR;
        Self {
            font_size,
            font_line_spacing,
            button_base_size,
            handle_area,
            grip: handle_area / GRIP_DIVISOR,
            min_side: MIN_SELECTION_SIDE,
            drag_threshold: DRAG_THRESHOLD,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::*;

    #[test]
    fn metrics_follow_the_f27_formulas() {
        let tokens = DesignTokens::default();
        let metrics = SelectionMetrics::from_tokens(&tokens);
        let base = f64::from(tokens.typography.base_size);
        assert_eq!(metrics.font_line_spacing, base * 1.2);
        assert_eq!(metrics.button_base_size, base * 1.2 * 2.2);
        assert_eq!(metrics.handle_area, base * 1.2 * 2.2 * 0.6);
        assert_eq!(metrics.grip, metrics.handle_area / 2.0);
        assert_eq!(metrics.min_side, 10.0);
        assert_eq!(metrics.drag_threshold, 3.0);
    }

    #[test]
    fn metrics_scale_with_the_typography_token() {
        let mut tokens = DesignTokens::default();
        tokens.typography.base_size = 20;
        let metrics = SelectionMetrics::from_tokens(&tokens);
        assert_eq!(metrics.font_line_spacing, 24.0);
        assert_eq!(metrics.button_base_size, 24.0 * 2.2);
    }
}
