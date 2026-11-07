//! Device-scoped renderer resources: one build per format, shared by every window.
//!
//! Pipelines, bind group layouts, and samplers describe how a device draws this
//! crate's command families, so they are per-device facts; the glyph atlas
//! texture, the frame target, and the uploaded textures are per-window facts. A
//! multi-window overlay child paid the WGSL compile and the pipeline build once
//! per window (the `perf.renderers_ready` leg of the startup clock), so the device
//! half is built once here and cloned into each window: wgpu handles are
//! `Arc`-backed clones that alias the same device resources, so cloning costs no
//! device work and no lifetime churn.
//!
//! The CPU raster path's presenter is deliberately kept out of the kit: a GPU-path
//! window would pay for a present pipeline it never draws with, and a CPU-path
//! window would pay for the vector pipelines it never uses.

use super::image_pipeline::image_pipeline;
use super::shadow::ShadowPipeline;
use super::target::msaa_sample_count;
use super::text_pipeline::text_pipeline;
use super::vector::VectorPipelines;

/// The per-device half of a renderer: built once, cloned into each window.
#[derive(Debug, Clone)]
pub(crate) struct DeviceKit {
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
    pub(crate) format: wgpu::TextureFormat,
    pub(crate) sample_count: u32,
    pub(crate) max_texture_dimension_2d: u32,
    pub(crate) vector: VectorPipelines,
    pub(crate) shadow: ShadowPipeline,
    pub(crate) text_layout: wgpu::BindGroupLayout,
    pub(crate) text_pipeline: wgpu::RenderPipeline,
    pub(crate) glyph_sampler: wgpu::Sampler,
    pub(crate) image_layout: wgpu::BindGroupLayout,
    pub(crate) image_pipeline: wgpu::RenderPipeline,
    pub(crate) image_sampler: wgpu::Sampler,
}

impl DeviceKit {
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> Self {
        let sample_count = msaa_sample_count(device);
        let text_layout = texture_bind_group_layout(device, "render-text-bindings");
        let image_layout = texture_bind_group_layout(device, "render-image-bindings");
        Self {
            device: device.clone(),
            queue: queue.clone(),
            format,
            sample_count,
            max_texture_dimension_2d: device.limits().max_texture_dimension_2d,
            vector: VectorPipelines::new(device, format, sample_count),
            shadow: ShadowPipeline::new(device, format, sample_count),
            text_pipeline: text_pipeline(device, format, sample_count, &text_layout),
            image_pipeline: image_pipeline(device, format, sample_count, &image_layout),
            glyph_sampler: linear_sampler(device, "render-glyph-sampler"),
            image_sampler: linear_sampler(device, "render-image-sampler"),
            text_layout,
            image_layout,
        }
    }
}

fn texture_bind_group_layout(device: &wgpu::Device, label: &str) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

fn linear_sampler(device: &wgpu::Device, label: &str) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some(label),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        ..wgpu::SamplerDescriptor::default()
    })
}
