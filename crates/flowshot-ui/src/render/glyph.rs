//! Pure glyph-quad placement: shaped glyph + atlas slot to text vertices.
//!
//! Split from the GPU half ([`super::text`]) so placement math is unit-testable
//! without a device. Glyph origins arrive as integer physical pixels (cosmic-text
//! quantizes subpixel offsets into its raster cache key), so at 1:1 atlas
//! sampling every texel lands exactly on a pixel - linear filtering then only
//! smooths the glyph's own coverage edges.

use cosmic_text::{SwashContent, SwashImage};

use super::atlas::AtlasSlot;
use super::color::srgb_to_linear;
use super::geom::f32_from_u32;
use super::text::GLYPH_PADDING;

/// Interleaved text vertex: `[pos xy, uv xy, tint rgba]`.
pub(crate) type TextVertex = [f32; 8];

/// Bytes per text vertex.
pub(crate) const TEXT_VERTEX_STRIDE: u64 = 8 * std::mem::size_of::<f32>() as u64;

/// Everything needed to place one rasterized glyph.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GlyphPlacement {
    /// Integer physical origin of the glyph (`PhysicalGlyph::x`).
    pub origin_x: i32,
    /// Integer physical baseline of the glyph (`PhysicalGlyph::y`).
    pub origin_y: i32,
    /// Ink offset left of the origin, px (swash placement).
    pub ink_left: i32,
    /// Ink offset above the baseline, px (swash placement).
    pub ink_top: i32,
    /// Ink extent in px.
    pub ink_width: u32,
    /// Ink extent in px.
    pub ink_height: u32,
    /// Atlas region including padding.
    pub slot: AtlasSlot,
    /// Padding inside the slot around the ink, px.
    pub slot_padding: u32,
    /// Atlas extent in texels (square).
    pub atlas_extent: u32,
    /// Premultiplied-linear tint multiplied with the atlas texel.
    pub tint: [f32; 4],
}

/// Builds the four corner vertices (top-left, bottom-left, top-right,
/// bottom-right) of one glyph quad; `None` for empty ink (spaces) or
/// non-finite origins.
pub(crate) fn glyph_quad(placement: &GlyphPlacement) -> Option<([TextVertex; 4], [u32; 6])> {
    if placement.ink_width == 0 || placement.ink_height == 0 {
        return None;
    }
    let x0 = f32_from_i32(placement.origin_x.saturating_add(placement.ink_left));
    let y0 = f32_from_i32(placement.origin_y.saturating_sub(placement.ink_top));
    let (w, h) = (
        f32_from_u32(placement.ink_width),
        f32_from_u32(placement.ink_height),
    );
    let atlas = f32_from_u32(placement.atlas_extent);
    let pad = f32_from_u32(placement.slot_padding);
    // Edge-to-edge UV span: the quad is exactly `ink` pixels wide, so pixel
    // centers land on texel centers (1:1 sampling, no half-texel shift) and
    // the zeroed padding border still guards linear filtering at the edges.
    let u0 = (f32_from_u32(placement.slot.x) + pad) / atlas;
    let v0 = (f32_from_u32(placement.slot.y) + pad) / atlas;
    let u1 = (f32_from_u32(placement.slot.x) + pad + w) / atlas;
    let v1 = (f32_from_u32(placement.slot.y) + pad + h) / atlas;
    let tint = placement.tint;
    let mut vertices = [[0.0f32; 8]; 4];
    let corners = [
        (x0, y0, u0, v0),
        (x0, y0 + h, u0, v1),
        (x0 + w, y0, u1, v0),
        (x0 + w, y0 + h, u1, v1),
    ];
    for (vertex, corner) in vertices.iter_mut().zip(corners) {
        *vertex = [
            corner.0, corner.1, corner.2, corner.3, tint[0], tint[1], tint[2], tint[3],
        ];
    }
    Some((vertices, [0, 1, 2, 2, 1, 3]))
}

/// Glyph origins are `i32` physical pixels far inside f32's exact-integer
/// range.
#[allow(clippy::cast_precision_loss)]
fn f32_from_i32(value: i32) -> f32 {
    value as f32
}

/// Mask glyphs take the text color; color glyphs keep their own colors and
/// only inherit the text alpha.
pub(crate) fn tint_for(content: SwashContent, text_tint: [f32; 4]) -> [f32; 4] {
    match content {
        // Subpixel masks never occur (cosmic-text rasterizes Format::Alpha);
        // should one appear, treating it as a coverage mask is the safe
        // degradation.
        SwashContent::Mask | SwashContent::SubpixelMask => text_tint,
        SwashContent::Color => [1.0, 1.0, 1.0, text_tint[3]],
    }
}

/// Converts a rasterized glyph to premultiplied-linear RGBA texels with the
/// padding border zeroed.
pub(crate) fn glyph_rgba(image: &SwashImage) -> Vec<u8> {
    let ink_w = usize::try_from(image.placement.width).unwrap_or(0);
    let ink_h = usize::try_from(image.placement.height).unwrap_or(0);
    let pad = usize::try_from(GLYPH_PADDING).unwrap_or(1);
    let row = (ink_w + pad * 2) * 4;
    let mut rgba = vec![0u8; row * (ink_h + pad * 2)];
    match image.content {
        SwashContent::Mask => {
            for (index, &coverage) in image.data.iter().enumerate() {
                let (x, y) = (index % ink_w, index / ink_w);
                let offset = ((y + pad) * (ink_w + pad * 2) + x + pad) * 4;
                if let Some(texel) = rgba.get_mut(offset..offset + 4) {
                    texel.copy_from_slice(&[coverage, coverage, coverage, coverage]);
                }
            }
        }
        SwashContent::Color => {
            for (index, pixel) in image.data.as_chunks::<4>().0.iter().enumerate() {
                let (x, y) = (index % ink_w, index / ink_w);
                let offset = ((y + pad) * (ink_w + pad * 2) + x + pad) * 4;
                let Some(texel) = rgba.get_mut(offset..offset + 4) else {
                    continue;
                };
                let alpha = f32::from(pixel[3]) / 255.0;
                for (channel, source) in texel.iter_mut().zip(pixel) {
                    *channel = unit_to_byte(srgb_to_linear(f32::from(*source) / 255.0) * alpha);
                }
                texel[3] = pixel[3];
            }
        }
        SwashContent::SubpixelMask => {
            tracing::warn!("subpixel glyph masks are not supported; glyph renders empty");
        }
    }
    rgba
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // input clamped to [0, 1] before scaling
fn unit_to_byte(unit: f32) -> u8 {
    (unit.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::*;

    fn placement() -> GlyphPlacement {
        GlyphPlacement {
            origin_x: 100,
            origin_y: 200,
            ink_left: 2,
            ink_top: 20,
            ink_width: 10,
            ink_height: 24,
            slot: AtlasSlot {
                x: 32,
                y: 64,
                width: 12,
                height: 26,
            },
            slot_padding: 1,
            atlas_extent: 1024,
            tint: [0.5, 0.25, 0.125, 1.0],
        }
    }

    #[test]
    fn quad_sits_at_origin_plus_ink_offsets() {
        let (vertices, indices) = glyph_quad(&placement()).unwrap_or_else(|| panic!("quad"));
        // top-left = (100 + 2, 200 - 20)
        assert_eq!((vertices[0][0], vertices[0][1]), (102.0, 180.0));
        // bottom-right = top-left + ink extent
        assert_eq!((vertices[3][0], vertices[3][1]), (112.0, 204.0));
        assert_eq!(indices, [0, 1, 2, 2, 1, 3]);
        assert_eq!(&vertices[0][4..8], &[0.5, 0.25, 0.125, 1.0]);
    }

    #[test]
    fn uvs_span_ink_texels_edge_to_edge() {
        let (vertices, _) = glyph_quad(&placement()).unwrap_or_else(|| panic!("quad"));
        // u0 = (32 + 1) / 1024: the first ink texel's left edge, so the
        // first pixel center samples that texel's center exactly.
        assert_eq!(vertices[0][2], 33.0 / 1024.0);
        assert_eq!(vertices[0][3], 65.0 / 1024.0);
        // u1 = (32 + 1 + 10) / 1024
        assert_eq!(vertices[3][2], 43.0 / 1024.0);
        // v1 = (64 + 1 + 24) / 1024
        assert_eq!(vertices[3][3], 89.0 / 1024.0);
    }

    #[test]
    fn empty_ink_produces_no_quad() {
        let mut space = placement();
        space.ink_width = 0;
        assert!(glyph_quad(&space).is_none());
        let mut flat = placement();
        flat.ink_height = 0;
        assert!(glyph_quad(&flat).is_none());
    }
}
