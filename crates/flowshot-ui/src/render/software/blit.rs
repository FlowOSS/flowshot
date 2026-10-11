//! Image quads on the CPU path: frozen-frame blits and the magnifier's
//! sub-region sample, read straight from the registered bytes.
//!
//! A 1:1 quad takes the nearest-texel path, which is the backdrop's case and
//! costs one copy per destination pixel; only scaled quads pay bilinear
//! filtering. Registered sources are already decoded to the framebuffer's
//! linear-light space by [`SoftwareRenderer::upload`], so filtering happens in
//! linear light like the GPU sampler's and no texel ever crosses the transfer
//! function per frame — the table below is applied once per registered byte.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    reason = "physical-pixel and texel index math, clamped into range before each cast"
)]

use std::collections::HashMap;

use crate::render::color::srgb_to_linear;
use crate::render::geom::Rect;
use crate::render::list::{ImageCommand, TextureId};
use crate::render::software::ImageEntry;
use crate::render::software::buffer::{ClipMask, SoftwareFramebuffer, unit_to_byte};

/// Draws one image quad, or the magenta placeholder when its texture is
/// missing (the GPU renderer's failure contract).
pub(crate) fn blit(
    framebuffer: &mut SoftwareFramebuffer,
    clip: &[ClipMask],
    command: &ImageCommand,
    images: &HashMap<TextureId, ImageEntry>,
) {
    let destination = command.dst;
    if destination.size.width <= 0.0 || destination.size.height <= 0.0 {
        return;
    }
    let fade = command.alpha.clamp(0.0, 1.0);
    let (x_start, y_start) = (
        destination.origin.x.floor().max(0.0) as i32,
        destination.origin.y.floor().max(0.0) as i32,
    );
    let (x_end, y_end) = (
        destination.right().ceil().max(0.0) as i32,
        destination.bottom().ceil().max(0.0) as i32,
    );

    let Some(source) = images.get(&command.texture) else {
        tracing::error!(
            texture = command.texture.raw(),
            "cpu renderer drew the magenta placeholder for a missing texture",
        );
        let placeholder = unit_to_byte(fade);
        for y in y_start..y_end {
            for x in x_start..x_end {
                let coverage = framebuffer.coverage(clip, x, y);
                framebuffer.blend_covered(
                    framebuffer.offset(x, y),
                    scaled(coverage, fade),
                    [placeholder, 0, placeholder, placeholder],
                );
            }
        }
        return;
    };

    let (tex_w, tex_h) = source.size();
    let region = command
        .src
        .unwrap_or(Rect::from_parts(0.0, 0.0, tex_w as f32, tex_h as f32));
    let scale_x = region.size.width / destination.size.width;
    let scale_y = region.size.height / destination.size.height;
    #[expect(
        clippy::float_cmp,
        reason = "both scales derive from the same integer extents, so 1.0 is exact"
    )]
    let one_to_one = scale_x == 1.0 && scale_y == 1.0;

    let (fb_w, fb_h) = framebuffer.size();
    let (x_end, y_end) = (x_end.min(fb_w as i32), y_end.min(fb_h as i32));
    if one_to_one {
        blit_nearest(
            framebuffer,
            clip,
            command,
            source,
            fade,
            (x_start, x_end, y_start, y_end),
        );
        return;
    }
    blit_filtered(
        framebuffer,
        clip,
        command,
        source,
        fade,
        (x_start, x_end, y_start, y_end),
    );
}

/// The 1:1 case: with unit scales the texel for a destination pixel is the
/// source pixel offset by the rounded region delta, so addressing is integer
/// stepping over a source window hoisted out of the loop - no per-texel float
/// math and no per-texel bounds re-check. With nothing painted yet and every
/// texel opaque the blend degenerates to a copy, which is one memcpy per row.
pub(crate) fn blit_nearest(
    framebuffer: &mut SoftwareFramebuffer,
    clip: &[ClipMask],
    command: &ImageCommand,
    source: &ImageEntry,
    fade: f32,
    bounds: (i32, i32, i32, i32),
) {
    let (x_start, x_end, y_start, y_end) = bounds;
    let destination = command.dst;
    let (tex_w, tex_h) = source.size();
    let data = source.bytes();
    let region = command
        .src
        .unwrap_or(Rect::from_parts(0.0, 0.0, tex_w as f32, tex_h as f32));
    let (fb_w, _) = framebuffer.size();
    let shift_x = (region.origin.x - destination.origin.x + 0.5).floor() as i32;
    let shift_y = (region.origin.y - destination.origin.y + 0.5).floor() as i32;
    let (x_lo, x_hi) = (x_start.max(-shift_x), x_end.min(tex_w as i32 - shift_x));
    let (y_lo, y_hi) = (y_start.max(-shift_y), y_end.min(tex_h as i32 - shift_y));
    let clip_free = clip.is_empty();
    let opaque_fade = fade >= 1.0;
    if clip_free && opaque_fade && source.opaque() {
        let span = (x_hi - x_lo).max(0) as usize * 4;
        for y in y_lo..y_hi {
            let src = (y + shift_y) as usize * tex_w as usize * 4 + (x_lo + shift_x) as usize * 4;
            let dst = y as usize * fb_w as usize * 4 + x_lo as usize * 4;
            framebuffer.copy_row(dst, &data[src..][..span]);
        }
        return;
    }
    for y in y_lo..y_hi {
        let src_row = (y + shift_y) as usize * tex_w as usize * 4;
        let dst_row = y as usize * fb_w as usize * 4;
        for x in x_lo..x_hi {
            let src = src_row + (x + shift_x) as usize * 4;
            let at = dst_row + x as usize * 4;
            let clip_coverage = if clip_free {
                255
            } else {
                framebuffer.coverage(clip, x, y)
            };
            if clip_coverage == 0 {
                continue;
            }
            let coverage = if opaque_fade {
                clip_coverage
            } else {
                scaled(clip_coverage, fade)
            };
            framebuffer.blend_covered(
                at,
                coverage,
                [data[src], data[src + 1], data[src + 2], data[src + 3]],
            );
        }
    }
}

/// A scaled quad: bilinear filtering in linear light, the GPU sampler's shape.
pub(crate) fn blit_filtered(
    framebuffer: &mut SoftwareFramebuffer,
    clip: &[ClipMask],
    command: &ImageCommand,
    source: &ImageEntry,
    fade: f32,
    bounds: (i32, i32, i32, i32),
) {
    let (x_start, x_end, y_start, y_end) = bounds;
    let destination = command.dst;
    let (tex_w, tex_h) = source.size();
    let data = source.bytes();
    let region = command
        .src
        .unwrap_or(Rect::from_parts(0.0, 0.0, tex_w as f32, tex_h as f32));
    let scale_x = region.size.width / destination.size.width;
    let scale_y = region.size.height / destination.size.height;
    let clip_free = clip.is_empty();
    let opaque_fade = fade >= 1.0;
    for y in y_start..y_end {
        for x in x_start..x_end {
            let clip_coverage = if clip_free {
                255
            } else {
                framebuffer.coverage(clip, x, y)
            };
            if clip_coverage == 0 {
                continue;
            }
            let u = region.origin.x + (x as f32 + 0.5 - destination.origin.x) * scale_x;
            let v = region.origin.y + (y as f32 + 0.5 - destination.origin.y) * scale_y;
            let texel = bilinear(data, tex_w, tex_h, u - 0.5, v - 0.5);
            let coverage = if opaque_fade {
                clip_coverage
            } else {
                scaled(clip_coverage, fade)
            };
            framebuffer.blend_covered(framebuffer.offset(x, y), coverage, texel);
        }
    }
}

/// A coverage byte scaled by a uniform fade.
#[must_use]
pub(crate) fn scaled(coverage: u8, fade: f32) -> u8 {
    unit_to_byte(f32::from(coverage) * fade.clamp(0.0, 1.0))
}

/// The sRGB→linear byte table: the transfer function evaluated once per byte
/// value, applied when a source is registered rather than per texel per frame.
#[must_use]
pub(crate) fn linear_table() -> [u8; 256] {
    let mut table = [0u8; 256];
    for (index, slot) in (0..256).zip(table.iter_mut()) {
        *slot = unit_to_byte(srgb_to_linear(index as f32 / 255.0));
    }
    table
}

/// The bilinear blend of the four texels around texel-space `(x, y)`.
#[must_use]
pub(crate) fn bilinear(data: &[u8], width: u32, height: u32, x: f32, y: f32) -> [u8; 4] {
    let (x0f, y0f) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0f, y - y0f);
    let (x0, y0) = (x0f as i32, y0f as i32);
    let at = |tx: i32, ty: i32, channel: usize| -> f32 {
        let tx = tx.clamp(0, width as i32 - 1) as usize;
        let ty = ty.clamp(0, height as i32 - 1) as usize;
        f32::from(data[(ty * width as usize + tx) * 4 + channel])
    };
    let mix = |channel: usize| -> u8 {
        let top = at(x0, y0, channel) + (at(x0 + 1, y0, channel) - at(x0, y0, channel)) * fx;
        let bottom =
            at(x0, y0 + 1, channel) + (at(x0 + 1, y0 + 1, channel) - at(x0, y0 + 1, channel)) * fx;
        unit_to_byte((top + (bottom - top) * fy) / 255.0)
    };
    [mix(0), mix(1), mix(2), mix(3)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bilinear_clamps_to_edge_texels_instead_of_transparent_black() {
        // 2x2 opaque image: all texels have alpha 255
        let data = vec![
            10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255, 100, 110, 120, 255,
        ];
        // Sample negative coordinate (-0.5, -0.5) - must clamp to top-left texel (10, 20, 30, 255)
        let sample_neg = bilinear(&data, 2, 2, -0.5, -0.5);
        assert_eq!(sample_neg[3], 255, "clamped sample must stay opaque");
        assert_eq!(&sample_neg[..3], &[10, 20, 30]);

        // Sample outside bottom-right (2.5, 2.5) - must clamp to bottom-right texel
        let sample_pos = bilinear(&data, 2, 2, 2.5, 2.5);
        assert_eq!(sample_pos[3], 255);
        assert_eq!(&sample_pos[..3], &[100, 110, 120]);
    }

    #[test]
    fn blit_nearest_copies_with_non_zero_offsets_without_panic() {
        let mut fb = SoftwareFramebuffer::new(32, 32);
        let src_data = vec![255u8; 16 * 16 * 4];
        let entry = ImageEntry {
            width: 16,
            height: 16,
            data: src_data,
            opaque: true,
        };
        let mut images = HashMap::new();
        let tex_id = TextureId::new(42);
        images.insert(tex_id, entry);

        // Blit a 16x16 sub-image offset into destination at (8, 8)
        let cmd = ImageCommand {
            texture: tex_id,
            dst: Rect::from_parts(8.0, 8.0, 16.0, 16.0),
            src: Some(Rect::from_parts(0.0, 0.0, 16.0, 16.0)),
            alpha: 1.0,
        };
        blit(&mut fb, &[], &cmd, &images);

        // Verify destination received pixels at (8, 8)
        let at = (8 * 32 + 8) * 4;
        assert_eq!(&fb.data()[at..at + 4], &[255, 255, 255, 255]);
    }

    #[test]
    fn linear_table_matches_srgb_to_linear_transfer_function() {
        let table = linear_table();
        for (i, &slot) in table.iter().enumerate() {
            let expected = unit_to_byte(srgb_to_linear(i as f32 / 255.0));
            assert_eq!(slot, expected);
        }
    }
}
