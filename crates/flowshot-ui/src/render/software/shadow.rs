//! Drop shadows on the CPU path: the same analytic gaussian falloff the GPU
//! path's SDF shader evaluates, sampled over the shadow's bounding box.
//!
//! The signed distance of a rounded box gives the exact occluder outline; the
//! falloff is the gaussian of the outside distance at the token's sigma, which
//! is what the shader computes per fragment.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    reason = "physical-pixel index math, clamped into range before each cast"
)]

use crate::render::geom::{Point, Rect};
use crate::render::list::ShadowSpec;
use crate::render::software::buffer::{ClipMask, SoftwareFramebuffer, premultiplied, unit_to_byte};

/// The signed distance from a point to a rounded box's outline (negative
/// inside), the shader's `sd_round_box`.
#[must_use]
#[cfg_attr(not(test), expect(dead_code, reason = "analytic sdf reference helper"))]
pub(crate) fn sd_round_box(
    local_x: f32,
    local_y: f32,
    half_w: f32,
    half_h: f32,
    radius: f32,
) -> f32 {
    let qx = local_x.abs() - half_w + radius;
    let qy = local_y.abs() - half_h + radius;
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - radius
}

/// Draws a token-derived drop shadow for a rounded rect.
pub(crate) fn shadow(
    framebuffer: &mut SoftwareFramebuffer,
    clip: &[ClipMask],
    rect: &Rect,
    radius: f32,
    spec: &ShadowSpec,
) {
    if spec.blur <= 0.0 {
        return;
    }
    let sigma = spec.blur * 0.5;
    let expand = sigma * 3.0;
    let half_w = rect.size.width * 0.5;
    let half_h = rect.size.height * 0.5;
    let radius = radius.max(0.0).min(half_w).min(half_h);
    let center = Point::new(
        rect.origin.x + half_w + spec.offset.x,
        rect.origin.y + half_h + spec.offset.y,
    );
    let color = premultiplied(&spec.color);
    let x_start = (center.x - half_w - expand).floor() as i32;
    let y_start = (center.y - half_h - expand).floor() as i32;
    let x_end = (center.x + half_w + expand).ceil() as i32;
    let y_end = (center.y + half_h + expand).ceil() as i32;
    let two_sigma_squared = 2.0 * sigma * sigma;
    let (fb_w, fb_h) = framebuffer.size();
    let x_lo = x_start.max(0).min(fb_w as i32);
    let x_hi = x_end.max(0).min(fb_w as i32);
    let y_lo = y_start.max(0).min(fb_h as i32);
    let y_hi = y_end.max(0).min(fb_h as i32);
    let clip_free = clip.is_empty();

    for y in y_lo..y_hi {
        let local_y = y as f32 + 0.5 - center.y;
        let qy = local_y.abs() - half_h + radius;
        let qy_pos_sq = qy.max(0.0).powi(2);
        let row_offset = y as usize * fb_w as usize * 4;

        for x in x_lo..x_hi {
            let coverage = if clip_free {
                255
            } else {
                framebuffer.coverage(clip, x, y)
            };
            if coverage == 0 {
                continue;
            }
            let local_x = x as f32 + 0.5 - center.x;
            let qx = local_x.abs() - half_w + radius;
            let at = row_offset + x as usize * 4;

            // Inside occluder fast path: distance is 0, falloff is exactly 1.0
            if qx <= 0.0 && qy <= 0.0 {
                framebuffer.blend_covered(at, coverage, color);
                continue;
            }

            let outside =
                ((qx.max(0.0).powi(2) + qy_pos_sq).sqrt() + qx.max(qy).min(0.0) - radius).max(0.0);
            if outside >= expand {
                continue;
            }
            let falloff = (-(outside * outside) / two_sigma_squared).exp();
            let scaled = unit_to_byte(f32::from(coverage) * falloff);
            framebuffer.blend_covered(at, scaled, color);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::render::color::Color;

    #[test]
    fn sd_round_box_signed_distances() {
        let (half_w, half_h, radius) = (20.0, 10.0, 4.0);
        // Point at center (0, 0): inside -> negative distance
        let d_center = sd_round_box(0.0, 0.0, half_w, half_h, radius);
        assert!(d_center < 0.0);

        // Point on flat horizontal edge: (0, 10) -> distance should be ~0.0
        let d_edge = sd_round_box(0.0, 10.0, half_w, half_h, radius);
        assert!(d_edge.abs() < 1e-4);

        // Point far outside: (30, 20) -> distance should be positive
        let d_out = sd_round_box(30.0, 20.0, half_w, half_h, radius);
        assert!(d_out > 0.0);
    }

    #[test]
    fn shadow_occluder_fast_path_and_boundary_blending() {
        let mut fb = SoftwareFramebuffer::new(64, 64);
        let rect = Rect::from_parts(20.0, 20.0, 24.0, 24.0);
        let spec = ShadowSpec {
            offset: Point::new(0.0, 0.0),
            blur: 10.0, // sigma = 5.0, expand = 15.0
            color: Color::from_rgba8(0, 0, 0, 255),
        };
        shadow(&mut fb, &[], &rect, 0.0, &spec);

        // Center pixel inside the occluder (32, 32) must be fully black with alpha 255
        let center_idx = (32 * 64 + 32) * 4;
        assert_eq!(&fb.data()[center_idx..center_idx + 4], &[0, 0, 0, 255]);

        // Far pixel outside (0, 0) must be completely untouched (0, 0, 0, 0)
        assert_eq!(&fb.data()[0..4], &[0, 0, 0, 0]);

        // Zero blur is a silent no-op
        let mut fb_zero = SoftwareFramebuffer::new(64, 64);
        let spec_zero = ShadowSpec { blur: 0.0, ..spec };
        shadow(&mut fb_zero, &[], &rect, 0.0, &spec_zero);
        assert_eq!(fb_zero.data(), &[0u8; 64 * 64 * 4]);
    }
}
