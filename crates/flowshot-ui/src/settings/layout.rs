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

use std::ops::Range;

use flowshot_core::tokens::DesignTokens;

use crate::egui_host::theme;

/// The label column's cap in em (the base typography size). Sized so the
/// longest shipped field label fits on one line at the default tokens (the
/// `label_column_fits_every_field_label` test measures this against the
/// vendored Inter, not against this comment).
const LABEL_MAX_EM: f32 = 22.0;
/// The most of the available width the label column may take (the control
/// column keeps the rest). Sized so the em cap still wins inside the
/// centered content column at the default window width.
const LABEL_MAX_RATIO: f32 = 0.48;
/// The control column's floor in em before the label column yields.
const CONTROL_MIN_EM: f32 = 6.0;
/// Section-card titles: em scale over the base size (semibold face).
const TITLE_SCALE: f32 = 1.25;
/// The content column's cap in em: section cards never stretch past a
/// readable measure on wide windows - the column centers instead (the
/// GNOME Settings / Zed boxed-list pattern; a full-width card at 1280px
/// leaves every control stranded in horizontal dead space).
const CONTENT_MAX_EM: f32 = 52.0;
/// Single-line text fields' cap in em (a path-length field fills the
/// control column; short values like an extension do not stretch).
const FIELD_MAX_EM: f32 = 24.0;
/// Combo boxes' cap in em (the selected value sets the honest width).
const COMBO_MAX_EM: f32 = 20.0;

/// A `u32` token step as points (byte-sized steps; oversized tokens fall
/// back instead of truncating silently - the theme projection's rule).
fn px(value: u32, fallback: u8) -> f32 {
    f32::from(u8::try_from(value).unwrap_or(fallback))
}

/// f32 points -> one component of egui 0.31+'s integer `Margin` (rounds;
/// the `as` cast saturates at the `i8` edge, so an oversized value clamps
/// instead of wrapping).
#[must_use]
pub(crate) fn margin_points(points: f32) -> i8 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "f32->i8 saturates by Rust cast semantics; margin steps are small whole pixels"
    )]
    let truncated = points.round() as i8;
    truncated
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

    /// The centered content column's width for `available` scroll width:
    /// the em cap on wide windows, the full width below it.
    #[must_use]
    pub fn content_width(self, available: f32) -> f32 {
        available.min(self.base_size * CONTENT_MAX_EM).max(0.0)
    }

    /// The centered content column's left offset for `available` scroll
    /// width (zero below the cap - narrow windows keep every pixel).
    #[must_use]
    pub fn content_offset(self, available: f32) -> f32 {
        (available - self.content_width(available)) * 0.5
    }

    /// Single-line text fields' width cap (the path field fills the
    /// control column; short values keep an honest width).
    #[must_use]
    pub fn field_max_width(self) -> f32 {
        self.base_size * FIELD_MAX_EM
    }

    /// Combo boxes' width cap.
    #[must_use]
    pub fn combo_max_width(self) -> f32 {
        self.base_size * COMBO_MAX_EM
    }

    /// The control column's left edge for `available` scroll width - the
    /// x every control in every row starts at (centering offset + card
    /// padding + label column + gutter).
    #[must_use]
    pub fn control_x(self, available: f32) -> f32 {
        self.content_offset(available)
            + self.content_left()
            + self.label_width(self.content_width(available) - 2.0 * self.card_padding())
            + self.gutter()
    }
}

/// The measured per-item geometry the palette swatch grid chunks with (all
/// widths in points, measured once per frame from the live egui style + font).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PaletteMetrics {
    /// One swatch cell's width: swatch button + item spacing + remove button.
    pub(crate) cell_w: f32,
    /// The horizontal item spacing between cells in a row.
    pub(crate) gap: f32,
    /// The "+ Add swatch" button's width.
    pub(crate) add_w: f32,
}

/// One planned row of the palette swatch grid.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PaletteRow {
    /// The palette indices of the swatches this row carries (empty for a
    /// trailing add-button-only row).
    pub(crate) swatches: Range<usize>,
    /// Whether the "+ Add swatch" button is this row's last cell.
    pub(crate) add_button: bool,
    /// The row's laid-out width (cells + gaps + the add button when present),
    /// for the containment asserts.
    pub(crate) width: f32,
}

/// The planned palette swatch grid: uniform rows of swatch cells with the
/// "+ Add swatch" button flowing as the last cell. The deterministic
/// replacement for `egui::horizontal_wrapped` - see [`plan_palette`].
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PaletteGrid {
    /// Swatch cells per row (always >= 1).
    pub(crate) cells_per_row: usize,
    /// The rows, top to bottom.
    pub(crate) rows: Vec<PaletteRow>,
}

/// The width of a row carrying `count` swatch cells with `count - 1` gaps
/// between them; zero for an empty row.
#[expect(
    clippy::cast_precision_loss,
    reason = "a swatch count is a small whole number; usize->f32 is exact far beyond any realizable row"
)]
fn palette_row_width(count: usize, cell_w: f32, gap: f32) -> f32 {
    if count == 0 {
        0.0
    } else {
        count as f32 * cell_w + (count - 1) as f32 * gap
    }
}

/// Plans the palette swatch grid for `swatches` entries within a
/// `control_w`-wide control column - the deterministic row chunking that
/// replaces `egui::horizontal_wrapped`. Two prior passes relied on the
/// wrapped layout inferring its width from the card's layout chain; it never
/// did, so the swatches escaped the card's right edge in one line.
///
/// A row of `k` cells spans `k*cell_w + (k-1)*gap`, so the most cells that
/// fit is `floor((control_w + gap) / (cell_w + gap))`, floored to a minimum
/// of 1: a column narrower than one cell still renders one cell per row
/// (the lone cell overflows the degenerate column alone) rather than dividing
/// by zero or stacking the overflow horizontally. Swatches chunk uniformly
/// into rows of that many; the add button then flows into the last row's
/// remaining slack, or onto its own row when the last swatch row is full.
/// Every row's width is `<= control_w` whenever `control_w >= max(cell_w,
/// add_w)` (every realizable window width - the control column's floor is
/// only breached below a ~430px window, where one cell degrades alone).
#[must_use]
pub(crate) fn plan_palette(
    control_w: f32,
    metrics: PaletteMetrics,
    swatches: usize,
) -> PaletteGrid {
    let PaletteMetrics { cell_w, gap, add_w } = metrics;
    let pitch = cell_w + gap;
    let cells_per_row = if pitch > 0.0 {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the quotient is a small non-negative cell count; a negative (degenerate) quotient saturates to 0 and the max(1) floor takes over"
        )]
        let count = ((control_w + gap) / pitch).floor() as usize;
        count.max(1)
    } else {
        1
    };
    let mut rows: Vec<PaletteRow> = Vec::new();
    let mut start = 0;
    while start < swatches {
        let end = (start + cells_per_row).min(swatches);
        rows.push(PaletteRow {
            swatches: start..end,
            add_button: false,
            width: palette_row_width(end - start, cell_w, gap),
        });
        start = end;
    }
    match rows.last_mut() {
        Some(last) if last.width + gap + add_w <= control_w => {
            last.width += gap + add_w;
            last.add_button = true;
        }
        _ => rows.push(PaletteRow {
            swatches: 0..0,
            add_button: true,
            width: add_w,
        }),
    }
    PaletteGrid {
        cells_per_row,
        rows,
    }
}
