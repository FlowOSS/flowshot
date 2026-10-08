//! Text rendering: cosmic-text shaping + swash rasterization through a
//! texture atlas.
//!
//! This is the crate's single text stack (atlas-first per
//! no custom SDF). glyphon could not be used: no glyphon release
//! pairs with the workspace's wgpu 0.20 pin (crates.io-verified in the root
//! manifest note), so the atlas lives here - the same architecture glyphon
//! implements, on the same swash rasterizer cosmic-text's [`SwashCache`]
//! exposes. Glyphs rasterize once into a fixed `Rgba8Unorm` atlas
//! (premultiplied linear texels: masks become coverage-white, color bitmaps
//! are linearized) and draw as tinted quads.
//!
//! Shaping runs per command per frame (cosmic-text `Buffer`s are cheap for
//! overlay-scale text); a shaped-buffer cache is a later polish concern if
//! profiling ever shows the need.

use std::collections::HashMap;

use cosmic_text::{
    Attrs, Buffer, CacheKey, Family, FontSystem, Metrics, Shaping, SwashCache, SwashImage,
};

use super::atlas::{AtlasSlot, ShelfAtlas};
use super::geom::Point;
use super::glyph::{GlyphPlacement, TextVertex, glyph_quad, glyph_rgba, tint_for};
use super::list::{TextAnchor, TextCommand};
use super::text_pipeline::{atlas_resources, text_pipeline};

/// The shaped block's extent (widest line x line-box height) - the
/// [`TextAnchor::Center`] metrics, measured on the SAME buffer the glyphs
/// rasterize from (the exact-centering contract of
/// `PaintSink::draw_text_centered`).
fn block_size(buffer: &Buffer) -> (f32, f32) {
    let mut width = 0.0_f32;
    let mut height = 0.0_f32;
    for run in buffer.layout_runs() {
        width = width.max(run.line_w);
        height = height.max(run.line_top + run.line_height);
    }
    (width, height)
}

/// Atlas edge length in texels (well under the 4096 device floor).
pub(crate) const ATLAS_DIM: u32 = 1024;
/// Zeroed texel border around each glyph so linear filtering never bleeds
/// across slots.
pub(crate) const GLYPH_PADDING: u32 = 1;

/// The font stack, glyph atlas, and text pipeline.
pub(crate) struct TextStack {
    font_system: FontSystem,
    swash_cache: SwashCache,
    atlas: ShelfAtlas,
    slots: HashMap<CacheKey, AtlasSlot>,
    reset_this_frame: bool,
    atlas_texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
}

impl std::fmt::Debug for TextStack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad("TextStack { .. }")
    }
}

/// The atlas-mutating half of [`TextStack`], split out so glyph images can be
/// borrowed from the swash cache while slots and packer are mutated (disjoint
/// borrows through one struct).
struct AtlasUpload<'a> {
    queue: &'a wgpu::Queue,
    texture: &'a wgpu::Texture,
    atlas: &'a mut ShelfAtlas,
    slots: &'a mut HashMap<CacheKey, AtlasSlot>,
    reset_this_frame: &'a mut bool,
}

impl AtlasUpload<'_> {
    fn ensure_slot(&mut self, key: CacheKey, image: &SwashImage) -> Option<AtlasSlot> {
        if let Some(slot) = self.slots.get(&key) {
            return Some(*slot);
        }
        let width = image.placement.width.checked_add(GLYPH_PADDING * 2)?;
        let height = image.placement.height.checked_add(GLYPH_PADDING * 2)?;
        let mut slot = self.atlas.allocate(width, height);
        if slot.is_none() && !*self.reset_this_frame {
            // Stale slots from before the reset may already be referenced by
            // quads emitted this frame; resetting mid-frame is a degraded,
            // warned path that only triggers past ~1000 distinct glyphs.
            *self.reset_this_frame = true;
            tracing::warn!("glyph atlas exhausted; resetting (glyphs re-rasterize on demand)");
            self.atlas.reset();
            self.slots.clear();
            slot = self.atlas.allocate(width, height);
        }
        let slot = slot?;
        let rgba = glyph_rgba(image);
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: slot.x,
                    y: slot.y,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            &rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.slots.insert(key, slot);
        Some(slot)
    }
}

impl TextStack {
    pub(crate) fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        sample_count: u32,
    ) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("render-text-bindings"),
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
        let (atlas_texture, bind_group) = atlas_resources(device, &bind_group_layout);
        Self {
            font_system: FontSystem::new(),
            swash_cache: SwashCache::new(),
            atlas: ShelfAtlas::new(ATLAS_DIM, ATLAS_DIM),
            slots: HashMap::new(),
            reset_this_frame: false,
            atlas_texture,
            bind_group,
            pipeline: text_pipeline(device, format, sample_count, &bind_group_layout),
        }
    }

    pub(crate) const fn pipeline(&self) -> &wgpu::RenderPipeline {
        &self.pipeline
    }

    pub(crate) const fn bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }

    /// Allows one atlas reset per frame.
    pub(crate) fn begin_frame(&mut self) {
        self.reset_this_frame = false;
    }

    /// Shapes `command` and appends its glyph quads to the staging buffers.
    /// Invalid metrics are logged and skipped (cosmic-text rejects a zero
    /// line height, and lib code never panics).
    pub(crate) fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        command: &TextCommand,
        vertices: &mut Vec<TextVertex>,
        indices: &mut Vec<u32>,
    ) {
        if !command.font_size.is_finite()
            || command.font_size <= 0.0
            || !command.line_height.is_finite()
            || command.line_height <= 0.0
        {
            tracing::error!(
                font_size = command.font_size,
                line_height = command.line_height,
                "invalid text metrics; command skipped"
            );
            return;
        }
        let mut buffer = Buffer::new(
            &mut self.font_system,
            Metrics::new(command.font_size, command.line_height),
        );
        if let Some(width) = command.max_width {
            buffer.set_size(Some(width.max(0.0)), None);
        }
        let mut attrs = Attrs::new();
        if let Some(family) = &command.family {
            attrs = attrs.family(Family::Name(family.as_str()));
        }
        if command.bold {
            attrs = attrs.weight(cosmic_text::Weight::BOLD);
        }
        buffer.set_text(&command.text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.font_system, false);
        let origin = match command.anchor {
            TextAnchor::TopLeft => command.position,
            TextAnchor::Center => {
                let (width, height) = block_size(&buffer);
                Point::new(
                    command.position.x - width / 2.0,
                    command.position.y - height / 2.0,
                )
            }
        };

        let Self {
            font_system,
            swash_cache,
            atlas,
            slots,
            reset_this_frame,
            atlas_texture,
            ..
        } = self;
        let mut upload = AtlasUpload {
            queue,
            texture: atlas_texture,
            atlas,
            slots,
            reset_this_frame,
        };
        let tint = command.color.premultiplied_linear();
        for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                let physical = glyph.physical((origin.x, origin.y + run.line_y), 1.0);
                let Some(image) = swash_cache
                    .get_image(font_system, physical.cache_key)
                    .as_ref()
                else {
                    continue;
                };
                if image.placement.width == 0 || image.placement.height == 0 {
                    continue;
                }
                let Some(slot) = upload.ensure_slot(physical.cache_key, image) else {
                    continue;
                };
                let placement = GlyphPlacement {
                    origin_x: physical.x,
                    origin_y: physical.y,
                    ink_left: image.placement.left,
                    ink_top: image.placement.top,
                    ink_width: image.placement.width,
                    ink_height: image.placement.height,
                    slot,
                    slot_padding: GLYPH_PADDING,
                    atlas_extent: ATLAS_DIM,
                    tint: tint_for(image.content, tint),
                };
                let Some((quad, quad_indices)) = glyph_quad(&placement) else {
                    continue;
                };
                let base = u32::try_from(vertices.len()).unwrap_or(u32::MAX);
                vertices.extend_from_slice(&quad);
                indices.extend(quad_indices.iter().map(|index| index + base));
            }
        }
    }
}
