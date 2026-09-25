//! Image quads: frozen-frame textures and sub-region sampling.
//!
//! Frames are uploaded once per capture (plan todo 15) as linear-filtered,
//! mip-free RGBA textures - frozen content is drawn 1:1 (physical-first
//! rule) or magnified by the magnifier (todo 17), both of which want linear
//! filtering without mip chains. A missing texture renders the magenta
//! placeholder and logs a tracing error (todo 14 failure path); magenta is a
//! diagnostic signal, not a design-token color.

use std::collections::HashMap;

use super::geom::Rect;
pub(crate) use super::image_pipeline::ImageVertex;
use super::image_pipeline::image_pipeline;
use super::list::TextureId;
use crate::error::UiError;

/// Raw RGBA8 pixel data for upload (row-major, top row first, 4 bytes/px).
#[derive(Debug, Clone, Copy)]
pub struct RgbaImage<'a> {
    /// Width in texels.
    pub width: u32,
    /// Height in texels.
    pub height: u32,
    /// Pixel bytes; must be exactly `width * height * 4`.
    pub data: &'a [u8],
}

impl RgbaImage<'_> {
    /// Validates dimensions and data length against the device texture limit.
    ///
    /// # Errors
    ///
    /// [`UiError::TextureDataLength`] on a byte-count mismatch,
    /// [`UiError::TextureTooLarge`] when zero-sized or beyond
    /// `max_texture_dimension_2d`.
    pub fn validate(&self, max_texture_dimension_2d: u32) -> Result<(), UiError> {
        let expected = usize::try_from(self.width)
            .unwrap_or(usize::MAX)
            .saturating_mul(usize::try_from(self.height).unwrap_or(usize::MAX))
            .saturating_mul(4);
        if expected != self.data.len() {
            return Err(UiError::TextureDataLength {
                width: self.width,
                height: self.height,
                expected,
                actual: self.data.len(),
            });
        }
        if self.width == 0
            || self.height == 0
            || self.width > max_texture_dimension_2d
            || self.height > max_texture_dimension_2d
        {
            return Err(UiError::TextureTooLarge {
                width: self.width,
                height: self.height,
                max: max_texture_dimension_2d,
            });
        }
        Ok(())
    }
}

#[derive(Debug)]
struct TextureEntry {
    bind_group: wgpu::BindGroup,
    width: u32,
    height: u32,
    _texture: wgpu::Texture,
}

/// Uploaded image textures plus the quad pipeline that samples them.
#[derive(Debug)]
pub struct TextureStore {
    entries: HashMap<TextureId, TextureEntry>,
    placeholder: TextureEntry,
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    max_texture_dimension_2d: u32,
}

impl TextureStore {
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        sample_count: u32,
    ) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("render-image-bindings"),
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
        });
        let pipeline = image_pipeline(device, format, sample_count, &bind_group_layout);
        // Linear filtering, no mips: frozen frames draw 1:1 or magnified.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("render-image-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..wgpu::SamplerDescriptor::default()
        });
        let max_texture_dimension_2d = device.limits().max_texture_dimension_2d;
        let placeholder = upload(
            device,
            queue,
            &bind_group_layout,
            &sampler,
            &RgbaImage {
                width: 1,
                height: 1,
                data: &[255, 0, 255, 255],
            },
            "render-texture-placeholder",
        );
        Self {
            entries: HashMap::new(),
            placeholder,
            pipeline,
            bind_group_layout,
            sampler,
            max_texture_dimension_2d,
        }
    }

    /// Uploads (or replaces) the texture registered under `id`.
    ///
    /// # Errors
    ///
    /// Validation failures per [`RgbaImage::validate`].
    pub fn insert(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        id: TextureId,
        image: &RgbaImage<'_>,
    ) -> Result<(), UiError> {
        image.validate(self.max_texture_dimension_2d)?;
        let entry = upload(
            device,
            queue,
            &self.bind_group_layout,
            &self.sampler,
            image,
            "render-texture",
        );
        self.entries.insert(id, entry);
        Ok(())
    }

    /// Drops a texture; later draws referencing it fall back to the
    /// placeholder.
    pub fn remove(&mut self, id: TextureId) {
        self.entries.remove(&id);
    }

    /// The registered size of `id` in texels, when present.
    #[must_use]
    pub fn dimensions(&self, id: TextureId) -> Option<(u32, u32)> {
        self.entries
            .get(&id)
            .map(|entry| (entry.width, entry.height))
    }

    /// The bind group to draw `id` with: the registered texture, or the
    /// magenta placeholder plus a tracing error (todo 14 failure path).
    pub(crate) fn resolve(&self, id: TextureId) -> &wgpu::BindGroup {
        if let Some(entry) = self.entries.get(&id) {
            &entry.bind_group
        } else {
            tracing::error!(
                texture_id = id.raw(),
                "image texture not registered; drawing magenta placeholder"
            );
            &self.placeholder.bind_group
        }
    }

    pub(crate) const fn pipeline(&self) -> &wgpu::RenderPipeline {
        &self.pipeline
    }
}

// Six parameters are inherent to wgpu resource plumbing: the device+queue
// context pair, the two bind-group ingredients, the pixel payload, and the
// debug label; a wrapper struct would only hide them.
fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    image: &RgbaImage<'_>,
    label: &str,
) -> TextureEntry {
    use wgpu::util::DeviceExt;

    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: image.width,
                height: image.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        image.data,
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    });
    TextureEntry {
        bind_group,
        width: image.width,
        height: image.height,
        _texture: texture,
    }
}

/// Normalized UV rect `(u0, v0, u1, v1)` for a pixel sub-region; `None`
/// samples the whole texture. Out-of-bounds regions clamp to the texture.
pub(crate) fn uv_rect(src: Option<Rect>, tex_width: u32, tex_height: u32) -> [f32; 4] {
    let (w, h) = (
        super::geom::f32_from_u32(tex_width),
        super::geom::f32_from_u32(tex_height),
    );
    let Some(src) = src else {
        return [0.0, 0.0, 1.0, 1.0];
    };
    let u0 = (src.origin.x / w).clamp(0.0, 1.0);
    let v0 = (src.origin.y / h).clamp(0.0, 1.0);
    let u1 = (src.right() / w).clamp(u0, 1.0);
    let v1 = (src.bottom() / h).clamp(v0, 1.0);
    [u0, v0, u1, v1]
}

/// The four corner vertices (top-left, bottom-left, top-right, bottom-right)
/// of an image quad and its triangle indices.
pub(crate) fn image_quad(dst: Rect, uv: [f32; 4]) -> ([ImageVertex; 4], [u32; 6]) {
    let (x0, y0) = (dst.origin.x, dst.origin.y);
    let (x1, y1) = (dst.right(), dst.bottom());
    let [u0, v0, u1, v1] = uv;
    (
        [
            [x0, y0, u0, v0],
            [x0, y1, u0, v1],
            [x1, y0, u1, v0],
            [x1, y1, u1, v1],
        ],
        [0, 1, 2, 2, 1, 3],
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::*;

    #[test]
    fn validate_rejects_length_mismatch_and_zero_size() {
        let image = RgbaImage {
            width: 2,
            height: 2,
            data: &[0; 15],
        };
        assert!(matches!(
            image.validate(4096),
            Err(UiError::TextureDataLength { .. })
        ));
        let empty = RgbaImage {
            width: 0,
            height: 4,
            data: &[],
        };
        assert!(matches!(
            empty.validate(4096),
            Err(UiError::TextureTooLarge { .. })
        ));
        let oversized_data = vec![0u8; 8192 * 4];
        let oversized = RgbaImage {
            width: 8192,
            height: 1,
            data: &oversized_data,
        };
        assert!(matches!(
            oversized.validate(4096),
            Err(UiError::TextureTooLarge { max: 4096, .. })
        ));
        let good = RgbaImage {
            width: 2,
            height: 2,
            data: &[0; 16],
        };
        assert!(good.validate(4096).is_ok());
    }

    #[test]
    fn uv_rect_defaults_to_the_full_texture() {
        assert_eq!(uv_rect(None, 640, 480), [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn uv_rect_maps_and_clamps_pixel_subregions() {
        let region = Rect::from_parts(160.0, 120.0, 320.0, 240.0);
        assert_eq!(uv_rect(Some(region), 640, 480), [0.25, 0.25, 0.75, 0.75]);
        let overflow = Rect::from_parts(320.0, 240.0, 640.0, 480.0);
        assert_eq!(uv_rect(Some(overflow), 640, 480), [0.5, 0.5, 1.0, 1.0]);
        let inverted = Rect::from_parts(100.0, 100.0, -50.0, -50.0);
        let [u0, v0, u1, v1] = uv_rect(Some(inverted), 640, 480);
        assert!(u1 >= u0 && v1 >= v0);
    }

    #[test]
    fn image_quad_corners_pair_dst_with_uv() {
        let (vertices, indices) = image_quad(
            Rect::from_parts(10.0, 20.0, 30.0, 40.0),
            [0.0, 0.0, 0.5, 1.0],
        );
        assert_eq!(vertices[0], [10.0, 20.0, 0.0, 0.0]);
        assert_eq!(vertices[3], [40.0, 60.0, 0.5, 1.0]);
        assert_eq!(indices, [0, 1, 2, 2, 1, 3]);
    }
}
