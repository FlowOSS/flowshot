//! The settings form grid's pure layout math (no egui types): the two-column
//! metrics every row helper and offscreen pixel assert derives from.
//!
//! The grid: a fixed-width label column (right-aligned), a gutter, and the
//! control column - all sized from the design tokens. The label column is
//! the smaller of an em cap (typography token) and a ratio of the available
//! width, so narrow windows yield label space to the controls instead of
//! collapsing them. [`FormMetrics`] is `Copy` and GPU-free; the egui-side
//! row primitives (`settings::tabs::form`) and the QA pixel asserts consume
//! the SAME numbers, which is what makes the rendered grid unit-testable.

use flowshot_core::tokens::DesignTokens;

use crate::egui_host::theme;

/// The label column's cap in em (the base typography size). Sized so the
/// longest shipped field label fits on one line at the default tokens (the
/// `label_column_fits_every_field_label` test measures this against the
/// vendored Inter, not against this comment).
const LABEL_MAX_EM: f32 = 22.0;
/// The most of the available width the label column may take (the control
/// column keeps the rest).
const LABEL_MAX_RATIO: f32 = 0.45;
/// The control column's floor in em before the label column yields.
const CONTROL_MIN_EM: f32 = 6.0;
/// Section-card titles: em scale over the base size (semibold face).
const TITLE_SCALE: f32 = 1.25;

/// A `u32` token step as points (byte-sized steps; oversized tokens fall
/// back instead of truncating silently - the theme projection's rule).
fn px(value: u32, fallback: u8) -> f32 {
    f32::from(u8::try_from(value).unwrap_or(fallback))
}

/// The token-derived two-column form metrics.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FormMetrics {
    base_size: f32,
    small: f32,
    medium: f32,
    large: f32,
    radius_medium: f32,
    radius_large: f32,
}

impl FormMetrics {
    /// Projects the design tokens into the form grid.
    #[must_use]
    pub fn from_tokens(tokens: &DesignTokens) -> Self {
        Self {
            base_size: px(tokens.typography.base_size, 14),
            small: px(tokens.spacing.small, 4),
            medium: px(tokens.spacing.medium, 8),
            large: px(tokens.spacing.large, 16),
            radius_medium: px(tokens.radii.medium, 4),
            radius_large: px(tokens.radii.large, 8),
        }
    }

    /// The base typography size (em for the grid's derived widths).
    #[must_use]
    pub fn base_size(self) -> f32 {
        self.base_size
    }

    /// The tight spacing step (row rhythm padding, title rules).
    #[must_use]
    pub fn small(self) -> f32 {
        self.small
    }

    /// The default spacing step (gutter, card gap, tab-bar gap).
    #[must_use]
    pub fn medium(self) -> f32 {
        self.medium
    }

    /// The generous spacing step (window margin, card padding).
    #[must_use]
    pub fn large(self) -> f32 {
        self.large
    }

    /// The window margin: the gap between the window edge and the content
    /// (the host frame projects this; the offscreen asserts measure it).
    #[must_use]
    pub fn window_margin(self) -> f32 {
        self.large
    }

    /// The section card's inner padding.
    #[must_use]
    pub fn card_padding(self) -> f32 {
        self.large
    }

    /// The section card's corner radius.
    #[must_use]
    pub fn card_radius(self) -> f32 {
        self.radius_large
    }

    /// The control corner radius (buttons, fields, pills at rest).
    #[must_use]
    pub fn control_radius(self) -> f32 {
        self.radius_medium
    }

    /// The label/control gutter (equals the horizontal item spacing the
    /// theme projects, so the row cells land exactly here).
    #[must_use]
    pub fn gutter(self) -> f32 {
        self.medium
    }

    /// The uniform control height (the theme's `interact_size.y`).
    #[must_use]
    pub fn control_height(self) -> f32 {
        theme::control_height(self.base_size, self.small)
    }

    /// The row pitch: control height plus the vertical item spacing.
    #[must_use]
    pub fn row_pitch(self) -> f32 {
        self.control_height() + self.small
    }

    /// The section-title font size (semibold face).
    #[must_use]
    pub fn title_size(self) -> f32 {
        self.base_size * TITLE_SCALE
    }

    /// The label-column width for `available` content width: the em cap,
    /// unless the ratio or the control column's floor demands less.
    #[must_use]
    pub fn label_width(self, available: f32) -> f32 {
        let cap = self.base_size * LABEL_MAX_EM;
        let ratio = available * LABEL_MAX_RATIO;
        let control_floor = available - self.control_min_width() - self.gutter();
        cap.min(ratio).min(control_floor).max(0.0)
    }

    /// The control column's floor width (the label column yields below
    /// this).
    #[must_use]
    pub fn control_min_width(self) -> f32 {
        self.base_size * CONTROL_MIN_EM
    }

    /// The card content's left edge (window margin + card padding).
    #[must_use]
    pub fn content_left(self) -> f32 {
        self.window_margin() + self.card_padding()
    }

    /// The control column's left edge for `available` card content width -
    /// the x every control in every row starts at.
    #[must_use]
    pub fn control_x(self, available: f32) -> f32 {
        self.content_left() + self.label_width(available) + self.gutter()
    }
}
