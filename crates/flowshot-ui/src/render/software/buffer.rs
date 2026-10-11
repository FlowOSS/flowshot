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
//! The framebuffer is **premultiplied, sRGB-encoded** RGBA8 — the byte space the
//! GPU path writes into its sRGB target, so the two paths agree on opaque token
//! fills byte for byte. Antialiasing is coverage-based rather than
//! multisample-based, so translucent overlaps blend in the encoded space
//! instead of linear light: the difference is confined to antialiased edges,
//! the same band the cross-rasterizer parity policy masks.
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

use crate::render::color::Color;

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

/// The framebuffer: premultiplied sRGB-encoded RGBA8, row-major, top row first.
#[derive(Debug)]
pub(crate) struct SoftwareFramebuffer {
    width: u32,
    height: u32,
    data: Vec<u8>,
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
        }
    }

    /// The framebuffer extent in physical pixels.
    #[must_use]
    pub(crate) const fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The pixel bytes.
    #[must_use]
    pub(crate) const fn data(&self) -> &[u8] {
        self.data.as_slice()
    }

    /// Makes every texel fully transparent.
    pub(crate) fn clear(&mut self) {
        self.data.fill(0);
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
    }
}

/// A [`Color`] as premultiplied sRGB-encoded RGBA8.
#[must_use]
pub(crate) fn premultiplied(color: &Color) -> [u8; 4] {
    let alpha = unit_to_byte(color.a);
    let scaled = |unit: f32| -> u8 {
        u8::try_from((u32::from(unit_to_byte(unit)) * u32::from(alpha)) / 255).unwrap_or(255)
    };
    [scaled(color.r), scaled(color.g), scaled(color.b), alpha]
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
    let [r, g, b, a] = premultiplied(color);
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
