//! Per-frame GPU staging buffers: growable vertex/index storage shared by
//! every draw family, uploaded once per frame.

use super::glyph::TextVertex;
use super::image::ImageVertex;
use super::shadow::ShadowVertex;
use super::tess::FlatVertex;

/// A growable GPU staging buffer, recreated when the payload outgrows it.
#[derive(Debug)]
pub(super) struct GpuBuffer {
    buffer: Option<wgpu::Buffer>,
    label: &'static str,
    usage: wgpu::BufferUsages,
}

impl GpuBuffer {
    pub(super) const fn new(label: &'static str, usage: wgpu::BufferUsages) -> Self {
        Self {
            buffer: None,
            label,
            usage,
        }
    }

    pub(super) fn upload<'a>(
        &'a mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bytes: &[u8],
    ) -> Option<&'a wgpu::Buffer> {
        if bytes.is_empty() {
            return None;
        }
        let Ok(size) = u64::try_from(bytes.len()) else {
            return None;
        };
        let needs_new = self
            .buffer
            .as_ref()
            .is_none_or(|buffer| buffer.size() < size);
        if needs_new {
            self.buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(self.label),
                size,
                usage: self.usage,
                mapped_at_creation: false,
            }));
        }
        let buffer = self.buffer.as_ref()?;
        queue.write_buffer(buffer, 0, bytes);
        Some(buffer)
    }
}

#[derive(Debug)]
pub(super) struct StagingBuffers {
    pub(super) flat_vertex: GpuBuffer,
    pub(super) flat_index: GpuBuffer,
    pub(super) image_vertex: GpuBuffer,
    pub(super) image_index: GpuBuffer,
    pub(super) shadow_vertex: GpuBuffer,
    pub(super) shadow_index: GpuBuffer,
    pub(super) text_vertex: GpuBuffer,
    pub(super) text_index: GpuBuffer,
}

/// One frame's CPU-side staging payloads, by vertex family.
pub(super) struct StagingData<'a> {
    pub(super) flat_vertices: &'a [FlatVertex],
    pub(super) flat_indices: &'a [u32],
    pub(super) image_vertices: &'a [ImageVertex],
    pub(super) image_indices: &'a [u32],
    pub(super) shadow_vertices: &'a [ShadowVertex],
    pub(super) shadow_indices: &'a [u32],
    pub(super) text_vertices: &'a [TextVertex],
    pub(super) text_indices: &'a [u32],
}

/// The uploaded GPU buffers a pass draws from (`None` = family unused).
pub(super) struct UploadedBuffers<'a> {
    pub(super) flat_vertex: Option<&'a wgpu::Buffer>,
    pub(super) flat_index: Option<&'a wgpu::Buffer>,
    pub(super) image_vertex: Option<&'a wgpu::Buffer>,
    pub(super) image_index: Option<&'a wgpu::Buffer>,
    pub(super) shadow_vertex: Option<&'a wgpu::Buffer>,
    pub(super) shadow_index: Option<&'a wgpu::Buffer>,
    pub(super) text_vertex: Option<&'a wgpu::Buffer>,
    pub(super) text_index: Option<&'a wgpu::Buffer>,
}

impl StagingBuffers {
    pub(super) fn new() -> Self {
        let vertex_usage = wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST;
        let index_usage = wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST;
        Self {
            flat_vertex: GpuBuffer::new("render-flat-vertices", vertex_usage),
            flat_index: GpuBuffer::new("render-flat-indices", index_usage),
            image_vertex: GpuBuffer::new("render-image-vertices", vertex_usage),
            image_index: GpuBuffer::new("render-image-indices", index_usage),
            shadow_vertex: GpuBuffer::new("render-shadow-vertices", vertex_usage),
            shadow_index: GpuBuffer::new("render-shadow-indices", index_usage),
            text_vertex: GpuBuffer::new("render-text-vertices", vertex_usage),
            text_index: GpuBuffer::new("render-text-indices", index_usage),
        }
    }

    pub(super) fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        data: &StagingData<'_>,
    ) -> UploadedBuffers<'_> {
        UploadedBuffers {
            flat_vertex: self.flat_vertex.upload(
                device,
                queue,
                bytemuck::cast_slice(data.flat_vertices),
            ),
            flat_index: self.flat_index.upload(
                device,
                queue,
                bytemuck::cast_slice(data.flat_indices),
            ),
            image_vertex: self.image_vertex.upload(
                device,
                queue,
                bytemuck::cast_slice(data.image_vertices),
            ),
            image_index: self.image_index.upload(
                device,
                queue,
                bytemuck::cast_slice(data.image_indices),
            ),
            shadow_vertex: self.shadow_vertex.upload(
                device,
                queue,
                bytemuck::cast_slice(data.shadow_vertices),
            ),
            shadow_index: self.shadow_index.upload(
                device,
                queue,
                bytemuck::cast_slice(data.shadow_indices),
            ),
            text_vertex: self.text_vertex.upload(
                device,
                queue,
                bytemuck::cast_slice(data.text_vertices),
            ),
            text_index: self.text_index.upload(
                device,
                queue,
                bytemuck::cast_slice(data.text_indices),
            ),
        }
    }
}
