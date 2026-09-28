//! The pixel magnifier (draft F27 `magnifierwidget.h` cites).
//!
//! A cursor-following VIEW aid (never a color-picker - the chrome and the
//! eyedropper tool own the palette/pick flows): it samples a 17x17 source-px window
//! (`m_magPixels = 8` radius) around the cursor from the frozen frame,
//! renders it at zoom 10x (170x170 physical px) offset 16px from the
//! cursor, and flips to the opposite side near screen edges (all four).
//! Two shape variants per `[editor].magnifier_shape`: square (border +
//! crosshair arms, Flameshot `drawMagnifier`) and circle (elliptic clip +
//! ring, `drawMagnifierCircle`). Toggled by `[editor].magnifier` plus the
//! in-session `L` key (lens).
//!
//! # ADDITIONS (BORROW-MODIFIED per the plan)
//!
//! - 1px pixel-grid lines at zoom >= [`GRID_MIN_ZOOM`] (Flameshot has no
//!   grid; wayshot/spectacle-class magnifiers do).
//! - An RGB + hex readout of the pixel under the crosshair (the wayshot
//!   `--color` equivalence): `#RRGGBB R,G,B` in a bar attached to the
//!   widget.
//!
//! # Deviations from the Flameshot source (documented per clean-room rule)
//!
//! - BOTH variants sample with the square's edge CLAMP (source window
//!   clamped into the frame, crosshair arms shifted by the clamp offset so
//!   they keep pointing at the cursor's true pixel). Flameshot's circle
//!   variant instead samples a black-padded screenshot - padding would
//!   fabricate black pixels at screen edges and break the readout's
//!   "pixel under the crosshair" contract.
//! - All geometry is PHYSICAL px (the physical-first rule, #4871/#4920
//!   family): at scale != 1 the widget stays 170 physical px, never 170
//!   logical px re-multiplied by the DPR.
//! - The zoomed pixels are built CPU-side (nearest-neighbor by
//!   construction) into [`MagnifierTexture`] and drawn as one image quad:
//!   the renderer's single image sampler is linear-filtered, which would
//!   smear a 10x magnified pixel grid. The CPU path also composites the
//!   pixel-effect layer (the "sample post-effect" seam) and yields
//!   the readout value for free.
//!
//! # Sampling source
//!
//! The installed editor frame ([`FramePixels`], the same read side the
//! eyedropper samples - identical conversion math, so the readout and an
//! eyedropper pick at the same position always agree). No frame or a
//! cursor outside the frame -> no magnifier (the binary layer decides the
//! stitched-vs-per-output production policy).

mod paint;
mod readout;

#[cfg(test)]
mod tests;

use flowshot_core::geometry::LogicalPoint;

use super::effect::{PixelEffect, frame_region};
use super::tool::FramePixels;
use super::{EditorState, MagnifierShape};

pub use paint::{MagnifierTexture, MagnifierView, magnifier_texture_id};

/// Source-px radius around the cursor (F27 `m_magPixels = 8`).
pub const MAG_PIXELS: i64 = 8;
/// Sampled window edge in source px (`2 * MAG_PIXELS + 1`, F27 `m_pixels`).
pub const WINDOW_PX: i64 = 2 * MAG_PIXELS + 1;
/// Magnification factor (F27 `magZoom = 10`).
pub const ZOOM: i64 = 10;
/// Rendered widget edge in physical px: `WINDOW_PX * ZOOM` = 170 (the
/// relationship is pinned by a unit test, not a cast).
pub const RENDERED_PX: f64 = 170.0;
/// Cursor-to-widget gap in physical px (F27 `m_magOffset = 16`).
pub const CURSOR_OFFSET: f64 = 16.0;
/// The pixel grid draws at zoom factors >= this (plan ADDITION).
pub const GRID_MIN_ZOOM: i64 = 8;
/// Crosshair-arm ink alpha (F27: `m_color.setAlpha(130)` on the uiColor).
pub const ARM_ALPHA: u8 = 130;

/// One sampled magnifier window: the 17x17 source pixels (row-major RGBA),
/// the pixel under the crosshair, and the edge-clamp shift the crosshair
/// arms render with (Flameshot's `offsetX`/`offsetY`, source px).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MagnifierSample {
    /// The sampled window, row-major `RGBA8888` (`WINDOW_PX^2 * 4` bytes).
    pub pixels: Vec<u8>,
    /// The pixel under the crosshair (the readout source).
    pub center: [u8; 4],
    /// Window clamp shift in source px: the cursor sits at window index
    /// `(MAG_PIXELS + arm_offset.0, MAG_PIXELS + arm_offset.1)`.
    pub arm_offset: (i32, i32),
}

impl MagnifierSample {
    /// The window pixel at column `i`, row `j` (transparent black outside
    /// the window - callers index `0..WINDOW_PX`).
    #[must_use]
    pub fn window_pixel(&self, i: i64, j: i64) -> [u8; 4] {
        let (Ok(i), Ok(j)) = (usize::try_from(i), usize::try_from(j)) else {
            return [0, 0, 0, 0];
        };
        let stride = usize::try_from(WINDOW_PX * 4).unwrap_or(68);
        let start = j.wrapping_mul(stride).wrapping_add(i * 4);
        self.pixels
            .get(start..start + 4)
            .map_or([0, 0, 0, 0], |px| [px[0], px[1], px[2], px[3]])
    }

    /// The readout text: `#RRGGBB R,G,B` (wayshot `--color` hex
    /// equivalence plus the RGB triplet the plan's ADDITION mandates).
    #[must_use]
    pub fn readout_text(&self) -> String {
        let [r, g, b, _] = self.center;
        format!("#{r:02X}{g:02X}{b:02X} {r},{g},{b}")
    }

    /// The center pixel's `#RRGGBB` hex (the stable tracing/QA token).
    #[must_use]
    pub fn center_hex(&self) -> String {
        let [r, g, b, _] = self.center;
        format!("#{r:02X}{g:02X}{b:02X}")
    }
}

/// Samples the magnifier window around `at` (global logical) from the
/// frozen frame with the pixel-effect layer composited on top
/// (later effects win - the paint order). `None` when the frame is missing,
/// smaller than the window, or the cursor is outside the frame.
pub(super) fn sample(
    frame: Option<&FramePixels>,
    effects: &[PixelEffect],
    at: LogicalPoint,
) -> Option<MagnifierSample> {
    let frame = frame?;
    let (width, height) = (i64::from(frame.width), i64::from(frame.height));
    if width < WINDOW_PX || height < WINDOW_PX {
        tracing::debug!(target: "flowshot_ui::editor", width, height, "magnifier frame smaller than window");
        return None;
    }
    // The eyedropper's exact conversion: global logical ->
    // frame-local physical, truncated. Readout/pick agreement by contract.
    let local_x = (at.x.0 - frame.origin.x.0) * frame.scale;
    let local_y = (at.y.0 - frame.origin.y.0) * frame.scale;
    if !(0.0..f64::from(frame.width)).contains(&local_x)
        || !(0.0..f64::from(frame.height)).contains(&local_y)
    {
        return None;
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "range-checked to [0, dimension) above"
    )]
    let (fx, fy) = (local_x as i64, local_y as i64);
    // Flameshot drawMagnifier's clamp: window top-left into the frame, the
    // overflow recorded as the crosshair-arm offset.
    let wx = (fx - MAG_PIXELS).clamp(0, width - WINDOW_PX);
    let wy = (fy - MAG_PIXELS).clamp(0, height - WINDOW_PX);
    let offset = (
        i32::try_from(fx - MAG_PIXELS - wx).unwrap_or(0),
        i32::try_from(fy - MAG_PIXELS - wy).unwrap_or(0),
    );
    let mut pixels = Vec::with_capacity(usize::try_from(WINDOW_PX * WINDOW_PX * 4).unwrap_or(1156));
    for j in 0..WINDOW_PX {
        for i in 0..WINDOW_PX {
            pixels.extend_from_slice(&sample_one(frame, effects, wx + i, wy + j));
        }
    }
    let center = {
        let ci = (fx - wx).clamp(0, WINDOW_PX - 1);
        let cj = (fy - wy).clamp(0, WINDOW_PX - 1);
        let start = usize::try_from((cj * WINDOW_PX + ci) * 4).unwrap_or(0);
        pixels
            .get(start..start + 4)
            .map_or([0, 0, 0, 0], |px| [px[0], px[1], px[2], px[3]])
    };
    Some(MagnifierSample {
        pixels,
        center,
        arm_offset: offset,
    })
}

/// One window pixel: the frozen frame's byte, overridden by every effect
/// whose baked region covers it (post-effect sampling - the pixel-effect
/// layer's seam).
fn sample_one(frame: &FramePixels, effects: &[PixelEffect], px: i64, py: i64) -> [u8; 4] {
    let mut rgba = frame_pixel(frame, px, py);
    for effect in effects {
        let Some(region) = frame_region(frame, effect.rect()) else {
            continue;
        };
        let (Ok(x), Ok(y)) = (u32::try_from(px), u32::try_from(py)) else {
            continue;
        };
        let (Some(dx), Some(dy)) = (x.checked_sub(region.x), y.checked_sub(region.y)) else {
            continue;
        };
        if dx >= region.w || dy >= region.h {
            continue;
        }
        let start = usize::try_from((u64::from(dy) * u64::from(region.w) + u64::from(dx)) * 4)
            .unwrap_or(usize::MAX);
        if let Some(pixel) = effect.pixels().get(start..start + 4) {
            rgba = [pixel[0], pixel[1], pixel[2], pixel[3]];
        }
    }
    rgba
}

/// The frame's RGBA byte at a physical position (transparent black outside
/// - the window clamp keeps positions in bounds; this guards the math).
fn frame_pixel(frame: &FramePixels, px: i64, py: i64) -> [u8; 4] {
    let (Ok(x), Ok(y)) = (usize::try_from(px), usize::try_from(py)) else {
        return [0, 0, 0, 0];
    };
    let stride = usize::try_from(frame.width).unwrap_or(0) * 4;
    let start = y.wrapping_mul(stride).wrapping_add(x * 4);
    frame
        .rgba
        .get(start..start + 4)
        .map_or([0, 0, 0, 0], |p| [p[0], p[1], p[2], p[3]])
}

impl EditorState {
    /// Whether the magnifier is visible (`[editor].magnifier` seed + the
    /// in-session toggle key).
    #[must_use]
    pub const fn magnifier_visible(&self) -> bool {
        self.magnifier_visible
    }

    /// Toggles the magnifier (the in-session key seam; the config seed is
    /// [`EditorState::configure`]).
    pub fn toggle_magnifier(&mut self) {
        self.magnifier_visible = !self.magnifier_visible;
        tracing::info!(
            target: "flowshot_ui::editor",
            visible = self.magnifier_visible,
            "magnifier toggled"
        );
    }

    /// Sets the magnifier visibility (settings/config seam).
    pub fn set_magnifier_visible(&mut self, visible: bool) {
        self.magnifier_visible = visible;
    }

    /// The configured magnifier shape (`[editor].magnifier_shape`).
    #[must_use]
    pub const fn magnifier_shape(&self) -> MagnifierShape {
        self.magnifier_shape
    }

    /// Samples the magnifier window at `at` from this editor's frame and
    /// effect layer (the headless/QA seam behind
    /// [`EditorState::paint_magnifier`]).
    #[must_use]
    pub fn magnifier_sample(&self, at: LogicalPoint) -> Option<MagnifierSample> {
        sample(self.frame.as_ref(), &self.effects, at)
    }
}
