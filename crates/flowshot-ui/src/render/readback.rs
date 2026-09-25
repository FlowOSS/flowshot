//! GPU-to-CPU readback for offscreen targets (parity harness, evidence
//! dumps). Live overlay frames never read back - this exists for tests and
//! golden fixtures.

use crate::error::UiError;

/// Copies an `Rgba8Unorm*` texture's level 0 into tightly packed RGBA bytes.
///
/// # Errors
///
/// [`UiError::BufferMap`] when the staging buffer mapping fails (device
/// lost).
pub fn read_texture_rgba(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, UiError> {
    let row = width * 4;
    let padded_row =
        row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("render-readback"),
        size: u64::from(padded_row) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("render-readback-encoder"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::ImageCopyTexture {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::ImageCopyBuffer {
            buffer: &buffer,
            layout: wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(padded_row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));

    let (sender, receiver) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _sent = sender.send(result);
        });
    device.poll(wgpu::Maintain::wait());
    receiver
        .recv()
        .map_err(|_| UiError::BufferMap(wgpu::BufferAsyncError))?
        .map_err(UiError::BufferMap)?;

    let mut pixels =
        Vec::with_capacity(usize::try_from(u64::from(width) * u64::from(height) * 4).unwrap_or(0));
    {
        let mapped = buffer.slice(..).get_mapped_range();
        let row = usize::try_from(row).unwrap_or(0);
        let padded = usize::try_from(padded_row).unwrap_or(0);
        for line in 0..usize::try_from(height).unwrap_or(0) {
            let row_start = line * padded;
            pixels.extend_from_slice(mapped.get(row_start..row_start + row).unwrap_or(&[]));
        }
    }
    buffer.unmap();
    Ok(pixels)
}
