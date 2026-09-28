//! The pure launch-resolution math: the preselect vocabulary,
//! its resolution against the output layout and the resolved cursor, and the
//! logical-rect <-> persisted-`Region` conversions. No state lives here -
//! [`super::LaunchState`] and the route funnel consume these functions.
//!
//! Clamp semantics (documented for scale != 1: everything here is GLOBAL
//! LOGICAL px; the physical conversion is the export path's
//! [`OutputLayout::crop_rects`], per-output scale, physical-first):
//!
//! - explicit coordinates (`--region WxH+X+Y`, persisted last region):
//!   intersected with the layout - the off-layout part of a requested rect
//!   cannot be captured, so it is cropped; no overlap = no preselect.
//! - cursor-centered (`--region WxH`): shifted into the layout preserving
//!   the requested size (the acceptance contract: the export dimensions
//!   stay EXACTLY `WxH`); a size larger than the layout crops to bounds.
//! - any side below the engine minimum (10x10) expands to it, so a
//!   seeded rect always satisfies the selection engine's invariants.

use flowshot_core::config::Region;
use flowshot_core::geometry::{LogicalPoint, LogicalRect, LogicalSize, OutputInfo, OutputLayout};

use crate::selection::{MIN_SELECTION_SIDE, fit_into_bounds};

/// What the overlay should preselect at launch (the `--region` /
/// `--last-region` vocabulary, already parsed - the grammar is the CLI's;
/// see the [`super`] mapping table).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Preselect {
    /// No preselection: a bare interactive launch.
    #[default]
    None,
    /// `--region WxH[+X+Y]`: a rect in global logical pixels. `origin:
    /// None` = no explicit coordinates: center (then clamp) at the resolved
    /// cursor - or defer to the first motion when unresolved.
    Region {
        /// The requested size (global logical px).
        size: LogicalSize,
        /// The requested top-left corner; `None` = center at cursor.
        origin: Option<LogicalPoint>,
    },
    /// `--region at-cursor`: the whole output under the cursor becomes the
    /// initial selection (the interactive form of the
    /// output-at-cursor behavior; a recorded decision - the binary layer's
    /// launch flows confirm).
    OutputAtCursor,
    /// `--last-region` / `capture last`: the persisted
    /// `[capture].last_region` (the binary layer reads the TOML; `None` =
    /// nothing persisted yet = launch without preselect).
    LastRegion(Option<Region>),
}

impl Preselect {
    /// The stable tracing token (QA asserts on these, never prose).
    #[must_use]
    pub const fn token(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Region {
                origin: Some(_), ..
            } => "region-explicit",
            Self::Region { origin: None, .. } => "region-centered",
            Self::OutputAtCursor => "output-at-cursor",
            Self::LastRegion(Some(_)) => "last-region",
            Self::LastRegion(None) => "last-region-absent",
        }
    }

    /// Resolves the preselect against the layout and the resolved cursor:
    /// either the rect to seed now, or the pending form for the
    /// `AwaitFirstMotion` deferral (the layer-3 cursor contract).
    #[must_use]
    pub fn resolve(&self, cursor: Option<LogicalPoint>, layout: &OutputLayout) -> InitialSelection {
        match *self {
            Self::None => InitialSelection::Ready(None),
            Self::Region { size, origin } => match (origin, cursor) {
                (Some(at), _) => InitialSelection::Ready(explicit_region(size, at, layout)),
                (None, Some(at)) => InitialSelection::Ready(centered_region(size, at, layout)),
                (None, None) => InitialSelection::Deferred(PendingPreselect::CenterRegion { size }),
            },
            Self::OutputAtCursor => match cursor {
                Some(at) => InitialSelection::Ready(output_rect_at(at, layout)),
                None => InitialSelection::Deferred(PendingPreselect::OutputAtCursor),
            },
            Self::LastRegion(persisted) => InitialSelection::Ready(persisted.and_then(|region| {
                explicit_region(
                    LogicalSize::from_raw(f64::from(region.width), f64::from(region.height)),
                    LogicalPoint::from_raw(f64::from(region.x), f64::from(region.y)),
                    layout,
                )
            })),
        }
    }
}

/// The resolved launch-time selection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InitialSelection {
    /// Seeds the selection engine immediately (`None` = launch without a
    /// preselect - a bare interactive session, or a request that did not
    /// overlap the layout).
    Ready(Option<LogicalRect>),
    /// The cursor is unresolved (`AwaitFirstMotion`): apply the
    /// pending form on the first pointer motion the overlay receives.
    Deferred(PendingPreselect),
}

/// A cursor-dependent preselect waiting for the first pointer motion.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PendingPreselect {
    /// Center the `--region` size at the first motion position.
    CenterRegion {
        /// The requested size (global logical px).
        size: LogicalSize,
    },
    /// Select the output containing the first motion position.
    OutputAtCursor,
}

impl PendingPreselect {
    /// The stable tracing token (QA asserts on these, never prose).
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::CenterRegion { .. } => "center-region",
            Self::OutputAtCursor => "output-at-cursor",
        }
    }

    /// Resolves the pending preselect at the first motion's clamped global
    /// position; `None` degrades to no preselect (warned by the caller -
    /// the first motion is the resolution point, never retried).
    #[must_use]
    pub fn apply(self, at: LogicalPoint, layout: &OutputLayout) -> Option<LogicalRect> {
        match self {
            Self::CenterRegion { size } => centered_region(size, at, layout),
            Self::OutputAtCursor => output_rect_at(at, layout),
        }
    }
}

/// The output containing the resolved cursor: what `flowshot capture screen`
/// (no arg) targets - the recorded BEHAVIOR (not a config flag) fixing
/// the capability Flameshot hard-blocks on Wayland (F8 `screengrabber`
/// L249-255). `None` when the cursor is unresolved
/// (`AwaitFirstMotion` - there is no overlay to await motion on for a
/// non-interactive capture) or lies outside every output; the calling
/// binary layer owns the fallback policy.
#[must_use]
pub fn output_at_cursor(
    layout: &OutputLayout,
    cursor: Option<LogicalPoint>,
) -> Option<&OutputInfo> {
    cursor.and_then(|at| layout.output_at(at))
}

/// An explicit-coordinates rect (or a persisted last region): minimum
/// enforced at the requested top-left, then intersected with the layout
/// (the off-layout part cannot be captured). `None` = no overlap.
fn explicit_region(
    size: LogicalSize,
    at: LogicalPoint,
    layout: &OutputLayout,
) -> Option<LogicalRect> {
    let rect = LogicalRect::from_parts(at, with_minimum(size));
    let clamped = layout.clamp_region_to_layout(rect);
    if clamped.is_none() {
        tracing::warn!(
            target: "flowshot_ui::launch",
            "preselect region does not overlap the layout; launching without preselect"
        );
    }
    clamped
}

/// A cursor-centered rect: minimum enforced symmetrically around the
/// cursor, then shifted into the layout bounds PRESERVING the size (the
/// export-dimensions acceptance); an oversized request crops to bounds.
fn centered_region(
    size: LogicalSize,
    cursor: LogicalPoint,
    layout: &OutputLayout,
) -> Option<LogicalRect> {
    let Some(bounds) = layout.union_bounds() else {
        tracing::warn!(
            target: "flowshot_ui::launch",
            "empty layout; launching without preselect"
        );
        return None;
    };
    let size = with_minimum(size);
    let origin = LogicalPoint::from_raw(
        cursor.x.0 - size.width.0 / 2.0,
        cursor.y.0 - size.height.0 / 2.0,
    );
    Some(fit_into_bounds(
        LogicalRect::from_parts(origin, size),
        Some(bounds),
    ))
}

/// The whole logical rect of the output containing `at` (the `at-cursor`
/// preselect); `None` when `at` lies in a layout gap or outside the layout.
fn output_rect_at(at: LogicalPoint, layout: &OutputLayout) -> Option<LogicalRect> {
    let rect = layout.output_at(at).map(|output| output.logical_rect);
    if rect.is_none() {
        tracing::warn!(
            target: "flowshot_ui::launch",
            "cursor lies outside every output; launching without preselect"
        );
    }
    rect
}

/// Expands any side below the selection engine's minimum (10x10)
/// so a seeded rect always satisfies the engine's invariants.
fn with_minimum(size: LogicalSize) -> LogicalSize {
    LogicalSize::from_raw(
        size.width.0.max(MIN_SELECTION_SIDE),
        size.height.0.max(MIN_SELECTION_SIDE),
    )
}

/// Rounds a logical selection rect into the persisted [`Region`] form
/// (i32/u32); `None` when a coordinate exceeds the integer range (the
/// caller warns and skips the write - never saturates a persisted rect).
pub(crate) fn region_of(rect: LogicalRect) -> Option<Region> {
    Some(Region {
        x: axis_of(rect.x.0)?,
        y: axis_of(rect.y.0)?,
        width: extent_of(rect.width.0)?,
        height: extent_of(rect.height.0)?,
    })
}

fn axis_of(value: f64) -> Option<i32> {
    let rounded = value.round();
    if !(rounded >= f64::from(i32::MIN) && rounded <= f64::from(i32::MAX)) {
        return None;
    }
    #[expect(clippy::cast_possible_truncation, reason = "range-checked above")]
    let cast = rounded as i32;
    Some(cast)
}

fn extent_of(value: f64) -> Option<u32> {
    let rounded = value.round();
    if !(rounded >= 0.0 && rounded <= f64::from(u32::MAX)) {
        return None;
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "range-checked above"
    )]
    let cast = rounded as u32;
    Some(cast)
}
