//! The F27 SECURE pixelate bake (`pixelatetool.cpp`
//! clean-room).
//!
//! # Why fringe pseudo-pixelation, not block averaging
//!
//! A classic mosaic (average each block down to one color, upscale) LEAKS
//! the block means - dictionary/ML attacks (Unredacter) recover text from
//! them. Flameshot's secure path therefore never reads the region interior
//! at all: it samples only the 1px fringe ring AROUND the region,
//! interpolates between the fringes, and drowns the result in seeded
//! gaussian noise. The interior contributes ZERO bytes to the output, so
//! there is nothing to recover - strictly stronger than average-downsample.
//! This algorithm is mandated EXACTLY; the insecure
//! downscale-upscale mosaic is dropped entirely - no
//! reversible pixelate code path exists anywhere in the crate.
//!
//! # Algorithm (F27 constants, all pinned by tests)
//!
//! 1. Output grid = `trunc(region_dim * 0.5 / (size + 1))` per axis; a zero
//!    on either axis is a NO-OP (the plan's 1x1-region failure path -
//!    Flameshot paints uninitialized memory there; we return `None`).
//! 2. Fringes: the 1px row/column just OUTSIDE each region side, falling
//!    back to the region's own edge pixel line when the side touches the
//!    frame border (Flameshot's `offset_top/bottom/left/right` rules).
//! 3. Per output pixel: sample each fringe at the pixel's relative position
//!    plus N(0, 5*size+1) sampling noise (clamped into the fringe), blend
//!    the horizontal (left/right) and vertical (top/bottom) interpolations,
//!    add the N(0, 0.1) color noise, quantize with truncation (Flameshot's
//!    `static_cast<int>(0xff * c)` + clamp), alpha = opaque.
//! 4. Nearest-neighbour upscale of the grid to the region size (Flameshot's
//!    `Qt::FastTransformation`).
//!
//! Flameshot's interpolation weights `(qMin(x, width-x)/width) -
//! (qMin(y, height-y)/height) + 0.5` are INTEGER divisions: both quotients
//! are provably 0 (the numerator never reaches the denominator), so
//! `weight_h = weight_v = 0.5` for every pixel - the blend below implements
//! that degenerate constant form directly (byte-identical behavior, no dead
//! arithmetic).
//!
//! The noise is pre-generated in canonical pixel order
//! ([`noise::NoiseBuffer`]) so any future SIMD/rayon path consumes the SAME
//! buffer and stays byte-identical by construction (the plan's Oracle-r4
//! clearance; the dependency freeze of this task keeps the shipped path
//! scalar - measured against the 4K/50ms gate in the perf test).

pub(in crate::editor) mod noise;

use super::tool::FramePixels;

/// The bake target: a physical-px rect inside a [`FramePixels`] view
/// (already clamped to the frame by the caller - every index in this module
/// relies on `x + w <= frame.width`, `y + h <= frame.height`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct BakeRegion {
    /// Left edge, physical px.
    pub x: u32,
    /// Top edge, physical px.
    pub y: u32,
    /// Width, physical px.
    pub w: u32,
    /// Height, physical px.
    pub h: u32,
}

/// The F27 output-grid formula: `trunc(dim * 0.5 / (size + 1))` per axis.
/// `None` when either axis collapses to zero (the 1x1-region no-op rule).
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the product is non-negative and bounded by the region dimension"
)]
pub(super) fn grid_size(region: BakeRegion, size: u32) -> Option<(u32, u32)> {
    // Flameshot: `0.5 / qMax(1, size() + 1)` - the DENOMINATOR is guarded.
    let factor = 0.5 / f64::from(size.saturating_add(1).max(1));
    let w = (f64::from(region.w) * factor) as u32;
    let h = (f64::from(region.h) * factor) as u32;
    (w > 0 && h > 0).then_some((w, h))
}

/// Bakes the secure pseudo-pixelation of `region`: the nearest-upscaled
/// mosaic, region-sized RGBA (the pixel-overlay buffer).
pub(super) fn bake_pixelate(frame: &FramePixels, region: BakeRegion, size: u32) -> Option<Vec<u8>> {
    let started = std::time::Instant::now();
    let (grid, grid_w, grid_h) = pixelate_grid(frame, region, size)?;
    let pixels = upscale_nearest(&grid, grid_w, grid_h, region.w, region.h);
    tracing::info!(
        target: "flowshot_ui::editor",
        kind = "pixelate",
        w = region.w,
        h = region.h,
        elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        "effect baked"
    );
    Some(pixels)
}

/// The grid stage alone (the PSNR acceptance test bicubic-upscales THIS,
/// the attacker's best smooth reconstruction of the mosaic).
/// One output-grid pixel's position: integer grid coordinates plus the
/// relative [0,1) offsets the fringe interpolation samples at.
#[derive(Debug, Clone, Copy)]
struct GridPosition {
    x: u32,
    y: u32,
    horizontal: f32,
    vertical: f32,
}

#[expect(
    clippy::cast_precision_loss,
    reason = "grid axes stay far below 2^24 for any real capture; the f32 math is the F27 parity form"
)]
pub(super) fn pixelate_grid(
    frame: &FramePixels,
    region: BakeRegion,
    size: u32,
) -> Option<(Vec<u8>, u32, u32)> {
    let (grid_w, grid_h) = grid_size(region, size)?;
    let fringes = Fringes::new(frame, region);
    let noise = noise::NoiseBuffer::generate(grid_w, grid_h, size);
    let mut grid = vec![0u8; grid_w as usize * grid_h as usize * 4];
    for x in 0..grid_w {
        for y in 0..grid_h {
            let at = GridPosition {
                x,
                y,
                horizontal: x as f32 / grid_w as f32,
                vertical: y as f32 / grid_h as f32,
            };
            let samples = fringes.sample_all(frame, &noise, at);
            let color_noise = noise.color(x, y);
            let out = &mut grid[out_index(x, y, grid_h)..][..4];
            for channel in 0..3 {
                // weight_h = weight_v = 0.5 (the degenerate F27 weights).
                let horizontal_mix = (1.0 - at.horizontal) * samples[2][channel]
                    + at.horizontal * samples[3][channel];
                let vertical_mix =
                    (1.0 - at.vertical) * samples[0][channel] + at.vertical * samples[1][channel];
                // 0.5 * (a + b) is Flameshot's exact weighted-sum form
                // (weight_h = weight_v = 0.5); f32::midpoint would round
                // differently by 1 ulp on carry - parity wins.
                #[expect(
                    clippy::manual_midpoint,
                    reason = "the F27 exact form: 0.5 * (h + v), not midpoint rounding"
                )]
                let blended = 0.5 * (horizontal_mix + vertical_mix) + color_noise;
                out[channel] = channel_byte(blended);
            }
            out[3] = 255;
        }
    }
    Some((grid, grid_w, grid_h))
}

fn out_index(x: u32, y: u32, grid_h: u32) -> usize {
    (x as usize * grid_h as usize + y as usize) * 4
}

/// The quantizer: Flameshot's `clamp(static_cast<int>(0xff * c), 0, 0xff)`
/// (truncation toward zero; Rust's saturating float cast also defines the
/// NaN path C++ leaves UB).
#[expect(
    clippy::cast_possible_truncation,
    reason = "truncation toward zero then clamp is the F27 quantizer"
)]
fn channel_byte(value: f32) -> u8 {
    let quantized = (255.0 * value) as i32;
    u8::try_from(quantized.clamp(0, 255)).unwrap_or(0)
}

/// Nearest-neighbour upscale (Flameshot `Qt::FastTransformation`):
/// `dst(dx, dy) <- grid(dx * grid_w / w, dy * grid_h / h)`. Rows are built
/// once per distinct grid row and replicated; partial blocks at the region
/// edges are the natural consequence of the integer mapping (the last
/// block run is shorter when the dimensions do not divide).
fn upscale_nearest(grid: &[u8], grid_w: u32, grid_h: u32, w: u32, h: u32) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let (grid_w, grid_h) = (grid_w as usize, grid_h as usize);
    let mut out = vec![0u8; w * h * 4];
    let mut row = vec![0u8; w * 4];
    let mut built_for = usize::MAX;
    for dy in 0..h {
        let gy = dy * grid_h / h;
        if gy != built_for {
            for dx in 0..w {
                let gx = dx * grid_w / w;
                let src = (gy * grid_w + gx) * 4;
                row[dx * 4..dx * 4 + 4].copy_from_slice(&grid[src..src + 4]);
            }
            built_for = gy;
        }
        out[dy * w * 4..(dy + 1) * w * 4].copy_from_slice(&row);
    }
    out
}

/// The four 1px fringe lines around the region (F27 `offset_*` rules: the
/// line just outside each side, or the region's own edge line when the side
/// sits on the frame border).
#[derive(Debug, Clone, Copy)]
pub(super) struct Fringes {
    /// Row index of the top fringe line.
    pub top_y: u32,
    /// Row index of the bottom fringe line.
    pub bottom_y: u32,
    /// Column index of the left fringe line.
    pub left_x: u32,
    /// Column index of the right fringe line.
    pub right_x: u32,
    region: BakeRegion,
}

impl Fringes {
    pub(super) fn new(frame: &FramePixels, region: BakeRegion) -> Self {
        let at_top = region.y == 0;
        let at_bottom = region.y + region.h == frame.height;
        let at_left = region.x == 0;
        let at_right = region.x + region.w == frame.width;
        Self {
            top_y: region.y.saturating_sub(u32::from(!at_top)),
            bottom_y: region.y + region.h - 1 + u32::from(!at_bottom),
            left_x: region.x.saturating_sub(u32::from(!at_left)),
            right_x: region.x + region.w - 1 + u32::from(!at_right),
            region,
        }
    }

    /// Samples all four fringes for one output pixel: `[top, bottom, left,
    /// right]`, each as [r, g, b] floats in [0,1] (Flameshot's `c.redF()`).
    #[expect(
        clippy::cast_precision_loss,
        reason = "fringe spans stay far below 2^24 for any real capture"
    )]
    fn sample_all(
        &self,
        frame: &FramePixels,
        noise: &noise::NoiseBuffer,
        at: GridPosition,
    ) -> [[f32; 3]; 4] {
        let mut samples = [[0.0f32; 3]; 4];
        for (fringe, sample) in samples.iter_mut().enumerate() {
            let (nx, ny) = noise.sample(at.x, at.y, fringe);
            // Top/bottom fringes span the region width (1 row); left/right
            // span its height (1 column) - the degenerate axis clamps to 0.
            let (fw, fh) = if fringe < 2 {
                (self.region.w, 1)
            } else {
                (1, self.region.h)
            };
            let sx = fringe_index(at.horizontal * fw as f32 + nx, fw);
            let sy = fringe_index(at.vertical * fh as f32 + ny, fh);
            *sample = self.pixel(frame, fringe, sx, sy);
        }
        samples
    }

    fn pixel(&self, frame: &FramePixels, fringe: usize, sx: u32, sy: u32) -> [f32; 3] {
        let region = self.region;
        let (x, y) = match fringe {
            0 => (region.x + sx, self.top_y),
            1 => (region.x + sx, self.bottom_y),
            2 => (self.left_x, region.y + sy),
            // The caller loop enumerates exactly 0..4.
            _ => (self.right_x, region.y + sy),
        };
        frame_rgb(frame, x, y)
    }
}

/// Flameshot's `std::clamp(static_cast<int>(pos + noise), 0, len - 1)`:
/// truncation toward zero, then clamp into the fringe (`len >= 1` by
/// construction - regions are at least 1px on both axes).
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "truncation toward zero matches static_cast<int>; the clamp makes the value a valid index"
)]
fn fringe_index(position: f32, len: u32) -> u32 {
    (position as i64).clamp(0, i64::from(len) - 1) as u32
}

/// Reads one frame pixel as [r, g, b] floats in [0,1]. In-bounds by
/// construction: callers pass only clamped region coordinates plus the
/// inward-or-border fringe offsets of [`Fringes::new`].
fn frame_rgb(frame: &FramePixels, x: u32, y: u32) -> [f32; 3] {
    let at = (y as usize * frame.width as usize + x as usize) * 4;
    let pixel = &frame.rgba[at..at + 4];
    [
        f32::from(pixel[0]) / 255.0,
        f32::from(pixel[1]) / 255.0,
        f32::from(pixel[2]) / 255.0,
    ]
}
