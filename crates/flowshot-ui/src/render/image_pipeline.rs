//! Image-quad pipeline: textured, premultiplied, stencil-clipped,
//! multisampled. Split from [`super::image`] (the texture store half).

use std::borrow::Cow;

use super::vector::content_stencil;

const IMAGE_WGSL: &str = r"
struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(0) @binding(0) var image_texture: texture_2d<f32>;
@group(0) @binding(1) var image_sampler: sampler;

@vertex
fn vs_main(@location(0) position: vec2<f32>, @location(1) uv: vec2<f32>) -> VsOut {
    var out: VsOut;
    out.clip = vec4<f32>(position, 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let texel = textureSample(image_texture, image_sampler, in.uv);
    // Atlas/frame content is stored premultiplied; the surface blends
    // premultiplied, so pass through.
    return texel;
}
";

/// Interleaved `[pos xy, uv uv]` image-quad vertex.
pub(crate) type ImageVertex = [f32; 4];

const IMAGE_VERTEX_ATTRIBUTES: &[wgpu::VertexAttribute] =
    &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2];
const IMAGE_VERTEX_STRIDE: u64 = 4 * std::mem::size_of::<f32>() as u64;

/// Builds the image-quad pipeline for `bind_group_layout` (texture binding 0,
/// sampler binding 1).
pub(crate) fn image_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    sample_count: u32,
    bind_group_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("render-image-layout"),
        bind_group_layouts: &[bind_group_layout],
        push_constant_ranges: &[],
    });
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("render-image.wgsl"),
        source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(IMAGE_WGSL)),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("render-image"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: "vs_main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: IMAGE_VERTEX_STRIDE,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: IMAGE_VERTEX_ATTRIBUTES,
            }],
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
            entry_point: "fs_main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview: None,
    })
}
