//! Drop shadows on the CPU path: the same analytic gaussian falloff the GPU
//! path's SDF shader evaluates, sampled over the shadow's bounding box.
//!
//! The signed distance of a rounded box gives the exact occluder outline; the
//! falloff is the gaussian of the outside distance at the token's sigma, which
//! is what the shader computes per fragment.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "physical-pixel index math, clamped into range before each cast"
)]

use crate::render::geom::{Point, Rect};
use crate::render::list::ShadowSpec;
use crate::render::software::buffer::{ClipMask, SoftwareFramebuffer, premultiplied, unit_to_byte};

/// The signed distance from a point to a rounded box's outline (negative
/// inside), the shader's `sd_round_box`.
#[must_use]
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
    for y in y_start..y_end {
        for x in x_start..x_end {
            let coverage = framebuffer.coverage(clip, x, y);
            if coverage == 0 {
                continue;
            }
            let local_x = x as f32 + 0.5 - center.x;
            let local_y = y as f32 + 0.5 - center.y;
            let outside = sd_round_box(local_x, local_y, half_w, half_h, radius).max(0.0);
            let falloff = (-(outside * outside) / two_sigma_squared).exp();
            if falloff <= 0.0 {
                continue;
            }
            let scaled = unit_to_byte(f32::from(coverage) * falloff);
            framebuffer.blend_covered(framebuffer.offset(x, y), scaled, color);
        }
    }
}
