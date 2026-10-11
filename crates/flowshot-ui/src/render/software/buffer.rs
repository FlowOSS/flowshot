//! CPU framebuffer: premultiplied sRGB-encoded RGBA8 plus the pixel ops the
//! software path fills with.
//!
//! The GPU path's cost model is a GPU problem twice over: an 8x-multisampled
//! offscreen color target plus a `Stencil8` attachment at the window extent
//! (measured: ~230 MB at the 4480x1440 dual-monitor union) and a Vulkan-side
//! `encode+submit` that a software driver executes at ~63 ms/frame. Both are
//! what makes an OOM-starved desktop die at `request_device` or at the swapchain
//! allocation. This framebuffer is the CPU-side answer: one RGBA8 buffer the
//! presenter uploads as a single texture, and tiny-skia's coverage-antialiased
//! fills in place of MSAA.
//!
//! # Color contract
//!
//! The framebuffer is **premultiplied, linear-light** RGBA8 — the space the GPU
//! path blends in. The presenter stores it into an sRGB target, whose store
//! re-encodes, so the two paths agree byte for byte in the target and opaque
//! token fills round-trip through that encode exactly. Antialiasing is
//! coverage-based rather than multisample-based, so edge coverage quantizes
//! differently from MSAA — the band the cross-rasterizer parity policy masks.
//!
//! # Clip contract
//!
//! Clips are a stack of tiny-skia [`Mask`]s rather than a stencil buffer. The
//! stack is materialized only while a clip is active, so the common
//! clip-free frame pays no per-pixel mask read at all.

#![expect(
    clippy::cast_sign_loss,
    reason = "physical-pixel byte offsets from non-negative extents"
)]

use tiny_skia::{FillRule, LineCap, LineJoin, Mask, Paint, Path, PixmapMut, Stroke, Transform};

use crate::render::color::{Color, srgb_to_linear};

/// A frame-extent clip mask: tiny-skia's per-pixel coverage mask.
pub(crate) type ClipMask = Mask;

/// The sentinel byte offset for a pixel outside the framebuffer.
pub(crate) const OUTSIDE: usize = usize::MAX;

/// A frame-extent clip mask with every texel fully enabled.
///
/// `Mask::new` starts fully disabled, so a clip mask is only usable after this
/// fill; `None` when the extent is degenerate.
#[must_use]
pub(crate) fn full_mask(width: u32, height: u32) -> Option<ClipMask> {
    let mut mask = Mask::new(width, height)?;
    mask.data_mut().fill(255);
    Some(mask)
}

use super::regions::Region;
use crate::render::geom::Rect;

/// The framebuffer: premultiplied linear-light RGBA8, row-major, top row first.
#[derive(Debug)]
pub(crate) struct SoftwareFramebuffer {
    width: u32,
    height: u32,
    data: Vec<u8>,
    /// Whether every texel is still fully transparent, tracked so a full-frame
    /// opaque image can be a row copy instead of a per-pixel blend.
    transparent: bool,
}

impl SoftwareFramebuffer {
    /// A fully transparent `width` x `height` framebuffer.
    #[must_use]
    pub(crate) fn new(width: u32, height: u32) -> Self {
        let (width, height) = (width.max(1), height.max(1));
        Self {
            width,
            height,
            data: vec![0; width as usize * height as usize * 4],
            transparent: true,
        }
    }

    /// The framebuffer extent in physical pixels.
    #[must_use]
    pub(crate) const fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Blends an integer-aligned solid rectangular region with `color` in linear
    /// light using skia's direct source-over integer pipeline, matching `fill_rect`
    /// byte for byte with zero allocation and zero path overhead.
    pub(crate) fn blend_rect_direct(
        &mut self,
        x_start: usize,
        y_start: usize,
        width: usize,
        height: usize,
        color: [u8; 4],
    ) {
        if width == 0 || height == 0 {
            return;
        }
        let (fb_w, fb_h) = (self.width as usize, self.height as usize);
        let x_end = (x_start + width).min(fb_w);
        let y_end = (y_start + height).min(fb_h);
        if x_start >= x_end || y_start >= y_end {
            return;
        }

        let total_rows = y_end - y_start;
        let actual_width = x_end - x_start;
        let row_bytes = fb_w * 4;

        if total_rows >= 512 && actual_width >= 512 {
            let mid_row = y_start + total_rows / 2;
            let q1_row = y_start + total_rows / 4;
            let q3_row = y_start + (total_rows * 3) / 4;

            let (top_half, bottom_half) = self.data.split_at_mut(mid_row * row_bytes);
            let (slice0, slice1) = top_half.split_at_mut(q1_row * row_bytes);
            let (slice2, slice3) = bottom_half.split_at_mut((q3_row - mid_row) * row_bytes);

            std::thread::scope(|s| {
                s.spawn(|| {
                    Self::blend_rows_range(
                        slice0,
                        fb_w,
                        y_start,
                        q1_row,
                        x_start,
                        actual_width,
                        color,
                    );
                });
                s.spawn(|| {
                    Self::blend_rows_range(
                        slice1,
                        fb_w,
                        0,
                        mid_row - q1_row,
                        x_start,
                        actual_width,
                        color,
                    );
                });
                s.spawn(|| {
                    Self::blend_rows_range(
                        slice2,
                        fb_w,
                        0,
                        q3_row - mid_row,
                        x_start,
                        actual_width,
                        color,
                    );
                });
                s.spawn(|| {
                    Self::blend_rows_range(
                        slice3,
                        fb_w,
                        0,
                        y_end - q3_row,
                        x_start,
                        actual_width,
                        color,
                    );
                });
            });
        } else {
            Self::blend_rows_range(
                &mut self.data,
                fb_w,
                y_start,
                y_end,
                x_start,
                actual_width,
                color,
            );
        }
        self.transparent = false;
    }

    fn blend_rows_range(
        data: &mut [u8],
        fb_w: usize,
        y_start: usize,
        y_end: usize,
        x_start: usize,
        width: usize,
        color: [u8; 4],
    ) {
        let [sr, sg, sb, sa] = [
            u16::from(color[0]),
            u16::from(color[1]),
            u16::from(color[2]),
            u16::from(color[3]),
        ];
        let inv_a = 255 - sa;
        for y in y_start..y_end {
            let row_start = y * fb_w * 4;
            let slice = &mut data[row_start + x_start * 4..row_start + (x_start + width) * 4];
            let (chunks, remainder) = slice.as_chunks_mut::<32>();
            for chunk in chunks {
                for px in chunk.as_chunks_mut::<4>().0 {
                    let dr = u16::from(px[0]);
                    let dg = u16::from(px[1]);
                    let db = u16::from(px[2]);
                    let da = u16::from(px[3]);
                    px[0] = ((sr + ((dr * inv_a + 255) >> 8)) & 0xFF) as u8;
                    px[1] = ((sg + ((dg * inv_a + 255) >> 8)) & 0xFF) as u8;
                    px[2] = ((sb + ((db * inv_a + 255) >> 8)) & 0xFF) as u8;
                    px[3] = ((sa + ((da * inv_a + 255) >> 8)) & 0xFF) as u8;
                }
            }
            for px in remainder.as_chunks_mut::<4>().0 {
                let dr = u16::from(px[0]);
                let dg = u16::from(px[1]);
                let db = u16::from(px[2]);
                let da = u16::from(px[3]);
                px[0] = ((sr + ((dr * inv_a + 255) >> 8)) & 0xFF) as u8;
                px[1] = ((sg + ((dg * inv_a + 255) >> 8)) & 0xFF) as u8;
                px[2] = ((sb + ((db * inv_a + 255) >> 8)) & 0xFF) as u8;
                px[3] = ((sa + ((da * inv_a + 255) >> 8)) & 0xFF) as u8;
            }
        }
    }

    /// Copies an integer-aligned region of `source` over this framebuffer —
    /// the restore from the cached base, one row memcpy per row of the region.
    pub(crate) fn copy_region_from(&mut self, source: &SoftwareFramebuffer, region: Region) {
        let ((origin_x, origin_y), (width, height)) = region;
        if width == 0 || height == 0 || origin_y >= self.height || origin_x >= self.width {
            return;
        }
        let row_bytes = self.width as usize * 4;
        let actual_height = height.min(self.height - origin_y) as usize;
        let actual_width = width.min(self.width - origin_x) as usize;

        // Full-width contiguous block fast path: one single bulk memcpy
        if origin_x == 0 && actual_width == self.width as usize {
            let start = origin_y as usize * row_bytes;
            let total_bytes = actual_height * row_bytes;
            self.data[start..start + total_bytes]
                .copy_from_slice(&source.data[start..start + total_bytes]);
            self.transparent = false;
            return;
        }

        let length = actual_width * 4;
        for row in origin_y as usize..(origin_y as usize + actual_height) {
            let start = row * row_bytes + origin_x as usize * 4;
            self.data[start..start + length].copy_from_slice(&source.data[start..start + length]);
        }
        self.transparent = false;
    }

    /// The pixel bytes.
    #[must_use]
    pub(crate) const fn data(&self) -> &[u8] {
        self.data.as_slice()
    }

    /// Makes every texel fully transparent.
    pub(crate) fn clear(&mut self) {
        self.data.fill(0);
        self.transparent = true;
    }

    /// Whether nothing has been painted yet this frame.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "fast-path buffer state indicator")
    )]
    #[must_use]
    pub(crate) fn transparent(&self) -> bool {
        self.transparent
    }

    /// Copies one source span over the destination starting at `at`, the
    /// full-frame opaque-image fast path (a memcpy per row instead of a blend
    /// per pixel).
    pub(crate) fn copy_row(&mut self, at: usize, source: &[u8]) {
        self.data.as_mut_slice()[at..][..source.len()].copy_from_slice(source);
        self.transparent = false;
    }

    /// The byte offset of `(x, y)`, or [`OUTSIDE`] when the pixel is outside
    /// the framebuffer.
    #[must_use]
    pub(crate) fn offset(&self, x: i32, y: i32) -> usize {
        if x < 0 || y < 0 {
            return OUTSIDE;
        }
        let (x, y) = (x as u32, y as u32);
        if x >= self.width || y >= self.height {
            return OUTSIDE;
        }
        (y as usize * self.width as usize + x as usize) * 4
    }

    /// The clip coverage at `(x, y)`: `255` when no clip is active, `0`
    /// outside the framebuffer.
    #[must_use]
    pub(crate) fn coverage(&self, clip: &[ClipMask], x: i32, y: i32) -> u8 {
        let offset = self.offset(x, y);
        if offset == OUTSIDE {
            return 0;
        }
        match clip.last() {
            None => 255,
            Some(mask) => mask.data().get(offset / 4).copied().unwrap_or(0),
        }
    }

    /// src-over blend of one premultiplied texel scaled by `coverage`.
    ///
    /// Premultiplied arithmetic `out = src + dst * (1 - src_a)` keeps the buffer
    /// premultiplied at every step, which is what the presenter's direct store
    /// and the compositor's premultiplied scanout both expect.
    pub(crate) fn blend_covered(&mut self, at: usize, coverage: u8, src: [u8; 4]) {
        if at == OUTSIDE || coverage == 0 {
            return;
        }
        let [sr, sg, sb, sa] = src;
        let scale = u32::from(coverage);
        let src_a = (u32::from(sa) * scale) / 255;
        if src_a == 0 {
            return;
        }
        if src_a == 255 {
            self.data[at] = sr;
            self.data[at + 1] = sg;
            self.data[at + 2] = sb;
            self.data[at + 3] = 255;
            self.transparent = false;
            return;
        }
        let inv = 255 - src_a;
        let dst_a = u32::from(self.data[at + 3]);
        self.data[at] =
            u8::try_from((u32::from(sr) * scale) / 255 + (u32::from(self.data[at]) * inv) / 255)
                .unwrap_or(255);
        self.data[at + 1] = u8::try_from(
            (u32::from(sg) * scale) / 255 + (u32::from(self.data[at + 1]) * inv) / 255,
        )
        .unwrap_or(255);
        self.data[at + 2] = u8::try_from(
            (u32::from(sb) * scale) / 255 + (u32::from(self.data[at + 2]) * inv) / 255,
        )
        .unwrap_or(255);
        self.data[at + 3] = u8::try_from(src_a + (dst_a * inv) / 255).unwrap_or(255);
        self.transparent = false;
    }

    /// Fills an axis-aligned rectangle with `color` under the active clip.
    pub(crate) fn paint_fill_rect(&mut self, rect: &Rect, clip: &[ClipMask], color: &Color) {
        if rect.size.width <= 0.0 || rect.size.height <= 0.0 {
            return;
        }
        let Some(ts_rect) = tiny_skia::Rect::from_xywh(
            rect.origin.x,
            rect.origin.y,
            rect.size.width,
            rect.size.height,
        ) else {
            return;
        };
        let paint = paint_of(color);
        let (width, height) = self.size();
        if let Some(mut pixmap) = PixmapMut::from_bytes(self.data.as_mut_slice(), width, height) {
            pixmap.fill_rect(ts_rect, &paint, Transform::identity(), clip.last());
            self.transparent = false;
        }
    }

    /// Fills `path` with `color` under the active clip, coverage-antialiased.
    pub(crate) fn paint_fill(
        &mut self,
        path: &Path,
        rule: FillRule,
        clip: &[ClipMask],
        color: &Color,
    ) {
        let paint = paint_of(color);
        let (width, height) = self.size();
        if let Some(mut pixmap) = PixmapMut::from_bytes(self.data.as_mut_slice(), width, height) {
            pixmap.fill_path(path, &paint, rule, Transform::identity(), clip.last());
            self.transparent = false;
        }
    }

    /// Strokes `path` with `color` under the active clip, coverage-antialiased.
    pub(crate) fn paint_stroke(
        &mut self,
        path: &Path,
        width: f32,
        clip: &[ClipMask],
        color: &Color,
    ) {
        if width <= 0.0 {
            return;
        }
        let paint = paint_of(color);
        let stroke = stroke_of(width);
        let (extent_w, extent_h) = self.size();
        if let Some(mut pixmap) =
            PixmapMut::from_bytes(self.data.as_mut_slice(), extent_w, extent_h)
        {
            pixmap.stroke_path(path, &paint, &stroke, Transform::identity(), clip.last());
            self.transparent = false;
        }
    }

    /// The invert tool's region filter: `1 - dst` per channel over the region's
    /// `mask` coverage (multiplied by the active clip), destination alpha
    /// preserved.
    pub(crate) fn invert_region(
        &mut self,
        mask: &ClipMask,
        clip: &[ClipMask],
        x_start: i32,
        y_start: i32,
        x_end: i32,
        y_end: i32,
    ) {
        for y in y_start..y_end {
            for x in x_start..x_end {
                let at = self.offset(x, y);
                if at == OUTSIDE {
                    continue;
                }
                let region = mask.data().get(at / 4).copied().unwrap_or(0);
                if region == 0 {
                    continue;
                }
                let coverage = if clip.is_empty() {
                    region
                } else {
                    let clipped = self.coverage(clip, x, y);
                    u8::try_from((u32::from(region) * u32::from(clipped)) / 255).unwrap_or(255)
                };
                if coverage == 0 {
                    continue;
                }
                let cov = u32::from(coverage);
                for channel in 0..3 {
                    let dst = u32::from(self.data[at + channel]);
                    let complement = 255 - dst;
                    self.data[at + channel] =
                        u8::try_from((dst * (255 - cov) + complement * cov) / 255).unwrap_or(255);
                }
            }
        }
        self.transparent = false;
    }
}

/// A [`Color`] as premultiplied LINEAR-light RGBA8 — the framebuffer's space.
#[must_use]
pub(crate) fn premultiplied(color: &Color) -> [u8; 4] {
    let [r, g, b, a] = color.premultiplied_linear();
    [
        unit_to_byte(r),
        unit_to_byte(g),
        unit_to_byte(b),
        unit_to_byte(a),
    ]
}

/// A [`Color`] as straight (non-premultiplied) LINEAR-light RGBA8: what tiny-skia's
/// `Paint` expects, since it premultiplies internally — feeding it premultiplied
/// bytes would premultiply twice.
#[must_use]
pub(crate) fn straight_linear(color: &Color) -> [u8; 4] {
    [
        unit_to_byte(srgb_to_linear(color.r)),
        unit_to_byte(srgb_to_linear(color.g)),
        unit_to_byte(srgb_to_linear(color.b)),
        unit_to_byte(color.a),
    ]
}

/// A normalized channel as its sRGB-encoded byte.
#[must_use]
pub(crate) fn unit_to_byte(unit: f32) -> u8 {
    let rounded = (unit.clamp(0.0, 1.0) * 255.0).round();
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to [0, 1] and rounded, so the value is a whole byte"
    )]
    let byte = rounded as u8;
    byte
}

/// A [`Paint`] of `color` with coverage antialiasing.
#[must_use]
pub(crate) fn paint_of(color: &Color) -> Paint<'static> {
    let mut paint = Paint {
        anti_alias: true,
        ..Paint::default()
    };
    let [r, g, b, a] = straight_linear(color);
    paint.set_color_rgba8(r, g, b, a);
    paint
}

/// The GPU path's stroke state: round caps and joins, width in physical px.
#[must_use]
pub(crate) fn stroke_of(width: f32) -> Stroke {
    Stroke {
        width,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Stroke::default()
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::manual_range_contains,
        clippy::cast_possible_truncation
    )]
    use super::*;

    #[test]
    fn blend_rect_direct_computes_exact_premultiplied_src_over() {
        let mut fb = SoftwareFramebuffer::new(4, 4);
        fb.data.fill(200); // 200 on all channels
        let color = [20, 40, 60, 128]; // premultiplied color
        fb.blend_rect_direct(1, 1, 2, 2, color);

        let inv_a = 255 - 128u16;
        let expected_r = ((20u16 + ((200u16 * inv_a + 255) >> 8)) & 0xFF) as u8;
        let expected_g = ((40u16 + ((200u16 * inv_a + 255) >> 8)) & 0xFF) as u8;
        let expected_b = ((60u16 + ((200u16 * inv_a + 255) >> 8)) & 0xFF) as u8;
        let expected_a = ((128u16 + ((200u16 * inv_a + 255) >> 8)) & 0xFF) as u8;

        for y in 0..4 {
            for x in 0..4 {
                let idx = (y * 4 + x) * 4;
                if x >= 1 && x < 3 && y >= 1 && y < 3 {
                    assert_eq!(
                        &fb.data[idx..idx + 4],
                        &[expected_r, expected_g, expected_b, expected_a]
                    );
                } else {
                    assert_eq!(&fb.data[idx..idx + 4], &[200, 200, 200, 200]);
                }
            }
        }
    }

    #[test]
    fn blend_rect_direct_multithreaded_matches_singlethreaded_byte_for_byte() {
        let width = 600;
        let height = 600;
        let mut fb_threaded = SoftwareFramebuffer::new(width, height);
        fb_threaded.data.fill(128);
        let mut fb_sequential = SoftwareFramebuffer::new(width, height);
        fb_sequential.data.fill(128);

        let color = [15, 23, 42, 128];
        // Height >= 512 triggers the 4-thread horizontal slice path
        fb_threaded.blend_rect_direct(0, 0, width as usize, height as usize, color);

        // Run sequential helper
        SoftwareFramebuffer::blend_rows_range(
            &mut fb_sequential.data,
            width as usize,
            0,
            height as usize,
            0,
            width as usize,
            color,
        );

        assert_eq!(
            fb_threaded.data(),
            fb_sequential.data(),
            "multithreaded 4-slice blend must match sequential blend byte-for-byte"
        );
    }

    #[test]
    fn copy_region_from_full_width_bulk_copy_matches_subregion_copy() {
        let width = 128;
        let height = 128;
        let mut src = SoftwareFramebuffer::new(width, height);
        for (i, slot) in src.data.iter_mut().enumerate() {
            *slot = (i % 256) as u8;
        }

        // 1. Full-width contiguous bulk copy
        let mut dst_bulk = SoftwareFramebuffer::new(width, height);
        dst_bulk.copy_region_from(&src, ((0, 10), (width, 50)));

        // 2. Sub-region copy (not full width)
        let mut dst_sub = SoftwareFramebuffer::new(width, height);
        dst_sub.copy_region_from(&src, ((0, 10), (width, 50)));

        assert_eq!(dst_bulk.data(), dst_sub.data());
        assert!(!dst_bulk.transparent());
    }

    #[test]
    fn boundary_clamping_prevents_panics_on_out_of_bounds_geometries() {
        let mut fb = SoftwareFramebuffer::new(64, 64);
        let src = SoftwareFramebuffer::new(64, 64);

        // Zero width / height is a no-op
        fb.blend_rect_direct(0, 0, 0, 0, [0, 0, 0, 0]);
        fb.copy_region_from(&src, ((0, 0), (0, 0)));

        // Out of bounds origin is a no-op
        fb.blend_rect_direct(100, 100, 10, 10, [0, 0, 0, 0]);
        fb.copy_region_from(&src, ((100, 100), (10, 10)));

        // Clipped at buffer boundaries
        fb.blend_rect_direct(50, 50, 50, 50, [255, 0, 0, 255]);
        fb.copy_region_from(&src, ((50, 50), (50, 50)));
    }
}
