//! The multisampled frame target: 4x MSAA color plus the `Stencil8` clip
//! attachment, recreated on resize. The pass resolves the color into the
//! caller's view (surface or offscreen texture).

use super::vector::STENCIL_FORMAT;
use crate::error::UiError;

/// MSAA sample count for the frame target: 8x when the device opted into
/// adapter-specific format features (finer edge coverage - both visually and
/// for the cross-rasterizer parity harness), else the WebGPU-guaranteed 4x.
/// Lyon emits exact geometry; multisample coverage is the AA source.
#[must_use]
pub(crate) fn msaa_sample_count(device: &wgpu::Device) -> u32 {
    if device
        .features()
        .contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES)
    {
        8
    } else {
        4
    }
}

#[derive(Debug)]
pub(super) struct FrameTarget {
    pub(super) msaa_view: wgpu::TextureView,
    pub(super) stencil_view: wgpu::TextureView,
    pub(super) size: (u32, u32),
    _msaa: wgpu::Texture,
    _stencil: wgpu::Texture,
}

impl FrameTarget {
    /// The target for `size`, recreated when the extent changed.
    pub(super) fn for_size(
        current: &mut Option<Self>,
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
        sample_count: u32,
        width: u32,
        height: u32,
    ) {
        if current
            .as_ref()
            .is_some_and(|target| target.size == (width, height))
        {
            return;
        }
        let extent = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let msaa = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("render-msaa-color"),
            size: extent,
            mip_level_count: 1,
            sample_count,
            dimension: wgpu::TextureDimension::D2,
            format: color_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let stencil = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("render-clip-stencil"),
            size: extent,
            mip_level_count: 1,
            sample_count,
            dimension: wgpu::TextureDimension::D2,
            format: STENCIL_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        *current = Some(Self {
            msaa_view: msaa.create_view(&wgpu::TextureViewDescriptor::default()),
            stencil_view: stencil.create_view(&wgpu::TextureViewDescriptor::default()),
            size: (width, height),
            _msaa: msaa,
            _stencil: stencil,
        });
    }
}

/// Validates a render-target extent against the device texture limit.
pub(super) fn validate_extent(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> Result<(), UiError> {
    let max = device.limits().max_texture_dimension_2d;
    if width == 0 || height == 0 || width > max || height > max {
        return Err(UiError::RenderTargetTooLarge { width, height, max });
    }
    Ok(())
}
