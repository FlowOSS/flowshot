//! The blur variant's bake ("blur variant = gaussian ...
//! (size -> radius mapping 10-12 parity)").
//!
//! Flameshot renders the region through a `QGraphicsBlurEffect` TWICE
//! ("multiple repeat for make blur effect stronger") with blur radii 10
//! then 12; the design distills that to `radius = clamp(size, 10, 12)`
//! (qBound parity) applied as two gaussian passes. The workspace's
//! `fast_image_resize` pin is NOT in the lockfile, so the gaussian is a
//! hand-rolled separable 2-pass convolution ("stack blur or 2-pass
//! gaussian" are the sanctioned choices): exact
//! normalized kernel, edge-extend sampling, u8 quantization between the
//! passes (Flameshot's 8-bit intermediate render). Deterministic by
//! construction (fixed f64 kernel, fixed summation order).
//!
//! SECURITY NOTE: blur is the aesthetic variant, NOT the secure redaction -
//! the secure path is the fringe pseudo-pixelation of [`super::pixelate`]
//! (the reversible mosaic was deliberately dropped; a gaussian blur of
//! sufficient radius carries no exact-inverse, but only pixelate has the
//! interior-never-sampled guarantee).

use super::pixelate::BakeRegion;
use super::tool::FramePixels;

/// The F27 radius clamp lower bound (`qBound(10, size, 12)`).
pub(super) const BLUR_RADIUS_MIN: u32 = 10;
/// The F27 radius clamp upper bound.
pub(super) const BLUR_RADIUS_MAX: u32 = 12;
/// Flameshot's "rendered twice" repeat count.
pub(super) const BLUR_PASSES: u32 = 2;

/// Bakes the two-pass gaussian blur of `region`: region-sized RGBA.
/// `None` only for an empty region (the caller clamps to >= 1px).
pub(super) fn bake_blur(frame: &FramePixels, region: BakeRegion, size: u32) -> Option<Vec<u8>> {
    if region.w == 0 || region.h == 0 {
        return None;
    }
    let started = std::time::Instant::now();
    let kernel = gaussian_kernel(f64::from(size.clamp(BLUR_RADIUS_MIN, BLUR_RADIUS_MAX)) / 2.0);
    let mut pixels = copy_region(frame, region);
    let mut scratch = vec![0u8; pixels.len()];
    // Two passes, each horizontal-then-vertical; every direction swaps the
    // result back into `pixels` (an even swap count lands it there).
    for _pass in 0..BLUR_PASSES {
        for horizontal in [true, false] {
            Convolution {
                width: region.w as usize,
                height: region.h as usize,
                kernel: &kernel,
                horizontal,
            }
            .run(&pixels, &mut scratch);
            std::mem::swap(&mut pixels, &mut scratch);
        }
    }
    tracing::info!(
        target: "flowshot_ui::editor",
        kind = "blur",
        w = region.w,
        h = region.h,
        elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        "effect baked"
    );
    Some(pixels)
}

/// The normalized 1D gaussian kernel: offsets `-half..=half` with
/// `half = ceil(3 * sigma)` (the 3-sigma support), weights summing to 1.0
/// up to f64 rounding (the normalization the acceptance test pins).
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "sigma is positive and finite (guarded); kernel offsets stay far below 2^24"
)]
pub(super) fn gaussian_kernel(sigma: f64) -> Vec<f64> {
    let sigma = if sigma.is_finite() && sigma > 0.0 {
        sigma
    } else {
        1.0
    };
    let half = (3.0 * sigma).ceil().max(1.0) as usize;
    let mut weights: Vec<f64> = (0..=2 * half)
        .map(|index| {
            let d = index as f64 - half as f64;
            (-(d * d) / (2.0 * sigma * sigma)).exp()
        })
        .collect();
    let sum: f64 = weights.iter().sum();
    for weight in &mut weights {
        *weight /= sum;
    }
    weights
}

/// Copies the region's pixels out of the frame (row-major RGBA).
fn copy_region(frame: &FramePixels, region: BakeRegion) -> Vec<u8> {
    let row_bytes = region.w as usize * 4;
    let frame_row = frame.width as usize * 4;
    let mut out = vec![0u8; row_bytes * region.h as usize];
    for dy in 0..region.h as usize {
        let src = (region.y as usize + dy) * frame_row + region.x as usize * 4;
        out[dy * row_bytes..(dy + 1) * row_bytes]
            .copy_from_slice(&frame.rgba[src..src + row_bytes]);
    }
    out
}

/// One separable convolution direction over a row-major RGBA buffer
/// (stride = `width` in BOTH directions), with clamped edge sampling; every
/// channel (including alpha) is convolved and rounded to u8 (the 8-bit
/// intermediate of Flameshot's double `scene.render`).
struct Convolution<'kernel> {
    width: usize,
    height: usize,
    kernel: &'kernel [f64],
    horizontal: bool,
}

impl Convolution<'_> {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the accumulation is clamped to [0,255] before the byte cast"
    )]
    fn run(&self, src: &[u8], dst: &mut [u8]) {
        let half = self.kernel.len() / 2;
        let limit = if self.horizontal {
            self.width
        } else {
            self.height
        } - 1;
        let mut acc = [0.0f64; 4];
        for y in 0..self.height {
            for x in 0..self.width {
                acc.fill(0.0);
                for (offset, weight) in self.kernel.iter().enumerate() {
                    let along = if self.horizontal { x } else { y };
                    let tapped = (along + offset).saturating_sub(half).min(limit);
                    let (sx, sy) = if self.horizontal {
                        (tapped, y)
                    } else {
                        (x, tapped)
                    };
                    let at = (sy * self.width + sx) * 4;
                    for channel in 0..4 {
                        acc[channel] += f64::from(src[at + channel]) * weight;
                    }
                }
                let at = (y * self.width + x) * 4;
                for channel in 0..4 {
                    dst[at + channel] = acc[channel].round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
}
