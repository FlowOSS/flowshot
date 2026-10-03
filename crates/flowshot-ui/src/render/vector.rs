//! Vector and clip pipelines: flat premultiplied-color triangles with a
//! stencil clip test.
//!
//! One WGSL passthrough shader serves three pipelines that differ only in
//! blend/write state:
//!
//! - **content**: draws fills/strokes; stencil `Equal` against the current
//!   clip depth (dynamic reference, so no pipeline variants per depth).
//! - **clip push / pop**: color writes masked off; stencil `Equal` against
//!   the parent depth with `IncrSaturate` / `DecrSaturate`, which maintains
//!   the invariant "stencil value == number of active clips covering this
//!   pixel" exactly - pops restore the parent depth, so sibling clips at the
//!   same level never leak into each other.

use std::borrow::Cow;

/// Stencil attachment format for clip stacks (core in wgpu 0.20; `S8_UINT`
/// is a Vulkan-required format).
pub(crate) const STENCIL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Stencil8;

/// Stencil state for content draws at the current clip depth (reference set
/// per draw via `set_stencil_reference`).
pub(crate) fn content_stencil() -> wgpu::DepthStencilState {
    wgpu::DepthStencilState {
        format: STENCIL_FORMAT,
        depth_write_enabled: Some(false),
        depth_compare: Some(wgpu::CompareFunction::Always),
        stencil: wgpu::StencilState {
            front: stencil_face(wgpu::StencilOperation::Keep),
            back: stencil_face(wgpu::StencilOperation::Keep),
            read_mask: 0xFF,
            write_mask: 0,
        },
        bias: wgpu::DepthBiasState::default(),
    }
}

fn stencil_face(pass_op: wgpu::StencilOperation) -> wgpu::StencilFaceState {
    wgpu::StencilFaceState {
        compare: wgpu::CompareFunction::Equal,
        fail_op: wgpu::StencilOperation::Keep,
        depth_fail_op: wgpu::StencilOperation::Keep,
        pass_op,
    }
}

fn clip_stencil(pass_op: wgpu::StencilOperation) -> wgpu::DepthStencilState {
    wgpu::DepthStencilState {
        format: STENCIL_FORMAT,
        depth_write_enabled: Some(false),
        depth_compare: Some(wgpu::CompareFunction::Always),
        stencil: wgpu::StencilState {
            front: stencil_face(pass_op),
            back: stencil_face(pass_op),
            read_mask: 0xFF,
            write_mask: 0xFF,
        },
        bias: wgpu::DepthBiasState::default(),
    }
}

const FLAT_WGSL: &str = r"
struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(@location(0) position: vec2<f32>, @location(1) color: vec4<f32>) -> VsOut {
    var out: VsOut;
    out.clip = vec4<f32>(position, 0.0, 1.0);
    out.color = color;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return in.color;
}
";

/// Interleaved `[pos xy, color rgba]` flat-vertex layout (matches
/// [`super::tess::FlatVertex`]).
pub(crate) const FLAT_VERTEX_ATTRIBUTES: &[wgpu::VertexAttribute] =
    &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4];
/// Bytes per flat vertex.
pub(crate) const FLAT_VERTEX_STRIDE: u64 = 6 * std::mem::size_of::<f32>() as u64;

/// Everything that distinguishes one flat-vertex pipeline from another
/// (shared by the vector, clip, and shadow pipelines).
pub(crate) struct PipelineSpec<'a> {
    pub format: wgpu::TextureFormat,
    pub sample_count: u32,
    pub write_mask: wgpu::ColorWrites,
    pub blend: wgpu::BlendState,
    pub depth_stencil: Option<wgpu::DepthStencilState>,
    pub label: &'a str,
}

/// The invert pipeline's blend state: `src * (1 - dst_color)` per color
/// channel (with the unit-white source vertex color this is the exact
/// linear-light complement) while the destination alpha passes through
/// untouched.
fn invert_blend() -> wgpu::BlendState {
    wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::OneMinusDst,
            dst_factor: wgpu::BlendFactor::Zero,
            operation: wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::Zero,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        },
    }
}

/// The flat-color pipelines (content, clip push/pop, region invert).
#[derive(Debug)]
pub(crate) struct VectorPipelines {
    pub content: wgpu::RenderPipeline,
    pub clip_push: wgpu::RenderPipeline,
    pub clip_pop: wgpu::RenderPipeline,
    pub invert: wgpu::RenderPipeline,
}

impl VectorPipelines {
    pub(crate) fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        sample_count: u32,
    ) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("render-flat.wgsl"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(FLAT_WGSL)),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("render-flat-layout"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });
        let spec = |write_mask, blend, depth_stencil, label| PipelineSpec {
            format,
            sample_count,
            write_mask,
            blend,
            depth_stencil,
            label,
        };
        Self {
            content: build_flat_pipeline(
                device,
                &layout,
                &module,
                &spec(
                    wgpu::ColorWrites::ALL,
                    wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
                    Some(content_stencil()),
                    "render-vector-content",
                ),
            ),
            clip_push: build_flat_pipeline(
                device,
                &layout,
                &module,
                &spec(
                    wgpu::ColorWrites::empty(),
                    wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
                    Some(clip_stencil(wgpu::StencilOperation::IncrementClamp)),
                    "render-clip-push",
                ),
            ),
            clip_pop: build_flat_pipeline(
                device,
                &layout,
                &module,
                &spec(
                    wgpu::ColorWrites::empty(),
                    wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
                    Some(clip_stencil(wgpu::StencilOperation::DecrementClamp)),
                    "render-clip-pop",
                ),
            ),
            invert: build_flat_pipeline(
                device,
                &layout,
                &module,
                &spec(
                    wgpu::ColorWrites::ALL,
                    invert_blend(),
                    Some(content_stencil()),
                    "render-vector-invert",
                ),
            ),
        }
    }
}

pub(crate) fn build_flat_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    spec: &PipelineSpec<'_>,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(spec.label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: FLAT_VERTEX_STRIDE,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: FLAT_VERTEX_ATTRIBUTES,
            })],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            unclipped_depth: false,
            polygon_mode: wgpu::PolygonMode::Fill,
            conservative: false,
        },
        depth_stencil: spec.depth_stencil.clone(),
        multisample: wgpu::MultisampleState {
            count: spec.sample_count,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: spec.format,
                blend: Some(spec.blend),
                write_mask: spec.write_mask,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}
