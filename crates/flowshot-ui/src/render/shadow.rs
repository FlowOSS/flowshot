//! Drop shadows: a single-pass signed-distance-field gaussian shader.
//!
//! The design allows "two-pass gaussian or shader"; the shader
//! path avoids an offscreen target per shadow. Each shadow is one quad
//! expanded to three sigma around the (offset) rounded rect; the fragment
//! shader evaluates the exact round-box SDF and applies a gaussian falloff,
//! so the result is resolution-independent and token-driven: sigma derives
//! from the token blur (`sigma = blur / 2`, the CSS convention where blur is
//! the ~2-sigma spread) and the color is the `#RRGGBBAA` shadow token.

use std::borrow::Cow;

use super::geom::Rect;
use super::list::ShadowSpec;
use super::vector::content_stencil;

/// Interleaved shadow vertex:
/// `[pos xy, local xy, half xy, radius+sigma, color rgba]`.
pub(crate) type ShadowVertex = [f32; 12];

/// Bytes per shadow vertex.
pub(crate) const SHADOW_VERTEX_STRIDE: u64 = 12 * std::mem::size_of::<f32>() as u64;

const SHADOW_VERTEX_ATTRIBUTES: &[wgpu::VertexAttribute] = &wgpu::vertex_attr_array![
    0 => Float32x2,
    1 => Float32x2,
    2 => Float32x2,
    3 => Float32x2,
    4 => Float32x4
];

const SHADOW_WGSL: &str = r"
struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) half_extent: vec2<f32>,
    @location(2) radius_sigma: vec2<f32>,
    @location(3) color: vec4<f32>,
};

@vertex
fn vs_main(
    @location(0) position: vec2<f32>,
    @location(1) local: vec2<f32>,
    @location(2) half_extent: vec2<f32>,
    @location(3) radius_sigma: vec2<f32>,
    @location(4) color: vec4<f32>,
) -> VsOut {
    var out: VsOut;
    out.clip = vec4<f32>(position, 0.0, 1.0);
    out.local = local;
    out.half_extent = half_extent;
    out.radius_sigma = radius_sigma;
    out.color = color;
    return out;
}

fn sd_round_box(p: vec2<f32>, half_extent: vec2<f32>, radius: f32) -> f32 {
    let q = abs(p) - half_extent + vec2<f32>(radius, radius);
    return length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let d = sd_round_box(in.local, in.half_extent, in.radius_sigma.x);
    let sigma = in.radius_sigma.y;
    let outside = max(d, 0.0);
    let falloff = exp(-(outside * outside) / (2.0 * sigma * sigma));
    // `color` arrives premultiplied; scaling rgb and a together keeps it so.
    return vec4<f32>(in.color.rgb * falloff, in.color.a * falloff);
}
";

/// The shadow render pipeline.
#[derive(Debug)]
pub(crate) struct ShadowPipeline {
    pub pipeline: wgpu::RenderPipeline,
}

impl ShadowPipeline {
    pub(crate) fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        sample_count: u32,
    ) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("render-shadow.wgsl"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(SHADOW_WGSL)),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("render-shadow-layout"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("render-shadow"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: SHADOW_VERTEX_STRIDE,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: SHADOW_VERTEX_ATTRIBUTES,
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..wgpu::PrimitiveState::default()
            },
            depth_stencil: Some(content_stencil()),
            multisample: wgpu::MultisampleState {
                count: sample_count,
                ..wgpu::MultisampleState::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self { pipeline }
    }
}

/// Gaussian sigma for a token blur radius (`sigma = blur / 2`).
pub(crate) fn sigma_for_blur(blur: f32) -> f32 {
    blur * 0.5
}

/// Quad expansion in px: three sigma, where the gaussian has decayed below
/// 1/255 of its peak for any token alpha.
pub(crate) fn expansion_for_sigma(sigma: f32) -> f32 {
    sigma * 3.0
}

/// Builds the four corner vertices (top-left, bottom-left, top-right,
/// bottom-right) of one shadow quad; `None` for degenerate input (invalid
/// rect, non-positive or non-finite blur).
pub(crate) fn shadow_quad(
    rect: Rect,
    radius: f32,
    spec: &ShadowSpec,
) -> Option<([ShadowVertex; 4], [u32; 6])> {
    if !rect.is_valid() || !spec.blur.is_finite() || spec.blur <= 0.0 {
        return None;
    }
    let sigma = sigma_for_blur(spec.blur);
    let expand = expansion_for_sigma(sigma);
    let center_x = rect.origin.x + rect.size.width * 0.5 + spec.offset.x;
    let center_y = rect.origin.y + rect.size.height * 0.5 + spec.offset.y;
    if !center_x.is_finite() || !center_y.is_finite() {
        return None;
    }
    let half = [rect.size.width * 0.5, rect.size.height * 0.5];
    let radius = radius
        .max(0.0)
        .min(half[0])
        .min(half[1])
        .min(f32::from(u16::MAX));
    let color = spec.color.premultiplied_linear();
    let extent = [half[0] + expand, half[1] + expand];
    let corners = [
        [-extent[0], -extent[1]],
        [-extent[0], extent[1]],
        [extent[0], -extent[1]],
        [extent[0], extent[1]],
    ];
    let mut vertices = [[0.0f32; 12]; 4];
    for (vertex, local) in vertices.iter_mut().zip(corners) {
        *vertex = [
            center_x + local[0],
            center_y + local[1],
            local[0],
            local[1],
            half[0],
            half[1],
            radius,
            sigma,
            color[0],
            color[1],
            color[2],
            color[3],
        ];
    }
    Some((vertices, [0, 1, 2, 2, 1, 3]))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::*;
    use crate::render::color::Color;
    use crate::render::geom::Point;

    fn spec(blur: f32) -> ShadowSpec {
        ShadowSpec {
            blur,
            offset: Point::new(0.0, 2.0),
            color: Color::from_hex_token("#00000080").unwrap_or_else(|| panic!("token")),
        }
    }

    #[test]
    fn quad_expands_three_sigma_and_applies_offset() {
        let rect = Rect::from_parts(100.0, 100.0, 40.0, 20.0);
        let (vertices, indices) =
            shadow_quad(rect, 4.0, &spec(8.0)).unwrap_or_else(|| panic!("quad"));
        // sigma = 4, expand = 12; center = (120, 110) + offset (0, 2).
        let expected_x = 120.0 + (20.0 + 12.0);
        let expected_y = 112.0 + (10.0 + 12.0);
        assert_eq!(vertices[3][0], expected_x);
        assert_eq!(vertices[3][1], expected_y);
        assert_eq!(vertices[0][0], 120.0 - 32.0);
        assert_eq!(vertices[0][1], 112.0 - 22.0);
        assert_eq!(vertices[0][6], 4.0, "radius");
        assert_eq!(vertices[0][7], 4.0, "sigma = blur/2");
        assert_eq!(indices, [0, 1, 2, 2, 1, 3]);
    }

    #[test]
    fn radius_clamps_to_half_the_smaller_side() {
        let rect = Rect::from_parts(0.0, 0.0, 40.0, 10.0);
        let (vertices, _) = shadow_quad(rect, 99.0, &spec(4.0)).unwrap_or_else(|| panic!("quad"));
        assert_eq!(vertices[0][6], 5.0);
    }

    #[test]
    fn degenerate_input_produces_no_quad() {
        let rect = Rect::from_parts(0.0, 0.0, 10.0, 10.0);
        assert!(shadow_quad(rect, 0.0, &spec(0.0)).is_none());
        assert!(shadow_quad(rect, 0.0, &spec(-1.0)).is_none());
        assert!(shadow_quad(rect, 0.0, &spec(f32::NAN)).is_none());
        assert!(shadow_quad(Rect::from_parts(f32::NAN, 0.0, 1.0, 1.0), 0.0, &spec(4.0)).is_none());
    }

    #[test]
    fn color_is_premultiplied_into_the_vertex() {
        let (vertices, _) = shadow_quad(Rect::from_parts(0.0, 0.0, 10.0, 10.0), 0.0, &spec(4.0))
            .unwrap_or_else(|| panic!("quad"));
        let expected = spec(4.0).color.premultiplied_linear();
        assert_eq!(&vertices[0][8..12], &expected);
    }
}
