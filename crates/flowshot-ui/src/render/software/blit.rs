//! Image quads on the CPU path: frozen-frame blits and the magnifier's
//! sub-region sample, read straight from the registered bytes.
//!
//! A 1:1 quad takes the nearest-texel path, which is the backdrop's case and
//! costs one copy per destination pixel; only scaled quads pay bilinear
//! filtering. Filtering happens in the encoded byte space rather than in
//! linear light, the documented deviation of the fallback's colorimetry.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "physical-pixel and texel index math, clamped into range before each cast"
)]

use std::collections::HashMap;

use crate::render::color::srgb_to_linear;
use crate::render::geom::Rect;
use crate::render::list::{ImageCommand, TextureId};
use crate::render::software::ImageSource;
use crate::render::software::buffer::{ClipMask, SoftwareFramebuffer, unit_to_byte};

/// Draws one image quad, or the magenta placeholder when its texture is
/// missing (the GPU renderer's failure contract).
pub(crate) fn blit(
    framebuffer: &mut SoftwareFramebuffer,
    clip: &[ClipMask],
    command: &ImageCommand,
    images: &HashMap<TextureId, ImageSource>,
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
    let data = source.bytes();

    for y in y_start..y_end {
        for x in x_start..x_end {
            let coverage = framebuffer.coverage(clip, x, y);
            if coverage == 0 {
                continue;
            }
            let u = region.origin.x + (x as f32 + 0.5 - destination.origin.x) * scale_x;
            let v = region.origin.y + (y as f32 + 0.5 - destination.origin.y) * scale_y;
            let texel = to_linear(if one_to_one {
                nearest(data, tex_w, tex_h, u, v)
            } else {
                bilinear(data, tex_w, tex_h, u - 0.5, v - 0.5)
            });
            framebuffer.blend_covered(framebuffer.offset(x, y), scaled(coverage, fade), texel);
        }
    }
}

/// A coverage byte scaled by a uniform fade.
#[must_use]
pub(crate) fn scaled(coverage: u8, fade: f32) -> u8 {
    unit_to_byte(f32::from(coverage) * fade.clamp(0.0, 1.0))
}

/// Image source bytes are premultiplied sRGB-encoded (the capture's space);
/// the framebuffer blends in linear light, so each rgb channel crosses the
/// transfer function before it composites. Alpha is linear by convention.
#[must_use]
pub(crate) fn to_linear(texel: [u8; 4]) -> [u8; 4] {
    let channel = |byte: u8| -> u8 { unit_to_byte(srgb_to_linear(f32::from(byte) / 255.0)) };
    [
        channel(texel[0]),
        channel(texel[1]),
        channel(texel[2]),
        texel[3],
    ]
}

/// The nearest texel at texel-space `(u, v)`, transparent outside the source.
#[must_use]
pub(crate) fn nearest(data: &[u8], width: u32, height: u32, u: f32, v: f32) -> [u8; 4] {
    let (x, y) = (u.floor(), v.floor());
    if x < 0.0 || y < 0.0 || x as u32 >= width || y as u32 >= height {
        return [0, 0, 0, 0];
    }
    let offset = (y as usize * width as usize + x as usize) * 4;
    [
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]
}

/// The bilinear blend of the four texels around texel-space `(x, y)`.
#[must_use]
pub(crate) fn bilinear(data: &[u8], width: u32, height: u32, x: f32, y: f32) -> [u8; 4] {
    let (x0f, y0f) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0f, y - y0f);
    let (x0, y0) = (x0f as i32, y0f as i32);
    let at = |tx: i32, ty: i32, channel: usize| -> f32 {
        if tx < 0 || ty < 0 || tx as u32 >= width || ty as u32 >= height {
            return 0.0;
        }
        f32::from(data[(ty as usize * width as usize + tx as usize) * 4 + channel])
    };
    let mix = |channel: usize| -> u8 {
        let top = at(x0, y0, channel) + (at(x0 + 1, y0, channel) - at(x0, y0, channel)) * fx;
        let bottom =
            at(x0, y0 + 1, channel) + (at(x0 + 1, y0 + 1, channel) - at(x0, y0 + 1, channel)) * fx;
        unit_to_byte((top + (bottom - top) * fy) / 255.0)
    };
    [mix(0), mix(1), mix(2), mix(3)]
}
