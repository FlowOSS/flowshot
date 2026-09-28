//! Renderer orchestration: display list to GPU frame.
//!
//! One [`Renderer`] owns every pipeline, the glyph atlas, the texture store,
//! and the multisampled frame target. Per frame it walks the [`DisplayList`]
//! once on the CPU - tessellating vector shapes (lyon), shaping text
//! (cosmic-text), expanding image and shadow quads - batching each family
//! into a shared staging buffer, then encodes a single render pass that
//! replays the recorded steps in list order. Antialiasing comes from the 4x
//! multisampled color target resolved into the caller's view; clipping from
//! an `Stencil8` attachment maintained by the clip push/pop steps.
//!
//! v1 redraws the full frame (damage tracking is explicitly optional in the
//! plan); [`FrameStats::cpu_time`] is the measured per-frame CPU cost the
//! <8 ms budget is checked against.

use std::time::{Duration, Instant};

use lyon::tessellation::VertexBuffers;

use super::frame_build::{FrameBuild, Step};
use super::glyph::TextVertex;
use super::image::{ImageVertex, TextureStore};
use super::list::DisplayList;
use super::pass::{PipelineRefs, draw_step};
use super::shadow::{ShadowPipeline, ShadowVertex};
use super::staging::{StagingBuffers, StagingData};
use super::target::{FrameTarget, msaa_sample_count, validate_extent};
use super::tess::{FlatVertex, Tessellator};
use super::text::TextStack;
use super::vector::VectorPipelines;
use crate::error::UiError;

/// Where one frame is rendered: a color attachment view (surface texture or
/// offscreen target) and its extent in physical pixels.
#[derive(Debug)]
pub struct RenderTarget<'a> {
    /// The attachment the MSAA frame resolves into.
    pub view: &'a wgpu::TextureView,
    /// Extent in physical pixels.
    pub width: u32,
    /// Extent in physical pixels.
    pub height: u32,
}

/// Per-frame measurements returned by [`Renderer::render`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameStats {
    /// Display-list commands consumed.
    pub commands: usize,
    /// Vertices staged across all families.
    pub vertices: usize,
    /// Draw calls encoded.
    pub draws: usize,
    /// CPU time for build + encode + submit (GPU work overlaps asynchronously).
    pub cpu_time: Duration,
}

/// The batched 2D renderer.
#[derive(Debug)]
pub struct Renderer {
    format: wgpu::TextureFormat,
    sample_count: u32,
    vector: VectorPipelines,
    shadow: ShadowPipeline,
    text: TextStack,
    textures: TextureStore,
    tessellator: Tessellator,
    target: Option<FrameTarget>,
    steps: Vec<Step>,
    flat: VertexBuffers<FlatVertex, u32>,
    image_vertices: Vec<ImageVertex>,
    image_indices: Vec<u32>,
    shadow_vertices: Vec<ShadowVertex>,
    shadow_indices: Vec<u32>,
    text_vertices: Vec<TextVertex>,
    text_indices: Vec<u32>,
    buffers: StagingBuffers,
}

impl Renderer {
    /// Builds the renderer for attachments of `format` (the surface format
    /// for live overlays, an sRGB format for offscreen targets).
    #[must_use]
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let sample_count = msaa_sample_count(device);
        Self {
            format,
            sample_count,
            vector: VectorPipelines::new(device, format, sample_count),
            shadow: ShadowPipeline::new(device, format, sample_count),
            text: TextStack::new(device, format, sample_count),
            textures: TextureStore::new(device, queue, format, sample_count),
            tessellator: Tessellator::new(),
            target: None,
            steps: Vec::new(),
            flat: VertexBuffers::new(),
            image_vertices: Vec::new(),
            image_indices: Vec::new(),
            shadow_vertices: Vec::new(),
            shadow_indices: Vec::new(),
            text_vertices: Vec::new(),
            text_indices: Vec::new(),
            buffers: StagingBuffers::new(),
        }
    }

    /// The color attachment format this renderer draws into.
    #[must_use]
    pub const fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    /// Read-only access to the uploaded image textures.
    #[must_use]
    pub const fn textures(&self) -> &TextureStore {
        &self.textures
    }

    /// Upload/remove frozen-frame and icon textures.
    #[must_use]
    pub fn textures_mut(&mut self) -> &mut TextureStore {
        &mut self.textures
    }

    /// Creates an offscreen color target matching this renderer's format,
    /// usable as [`RenderTarget::view`] and readable via
    /// [`super::read_texture_rgba`] (parity harness, golden fixtures).
    ///
    /// # Errors
    ///
    /// [`UiError::RenderTargetTooLarge`] when the extent exceeds the device
    /// texture limit.
    pub fn create_offscreen_target(
        &self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> Result<wgpu::Texture, UiError> {
        validate_extent(device, width, height)?;
        Ok(device.create_texture(&wgpu::TextureDescriptor {
            label: Some("render-offscreen-target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        }))
    }

    /// Renders one frame of `list` into `target`.
    ///
    /// # Errors
    ///
    /// [`UiError::RenderTargetTooLarge`] for zero or oversized extents.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &RenderTarget<'_>,
        list: &DisplayList,
    ) -> Result<FrameStats, UiError> {
        let started = Instant::now();
        validate_extent(device, target.width, target.height)?;
        FrameTarget::for_size(
            &mut self.target,
            device,
            self.format,
            self.sample_count,
            target.width,
            target.height,
        );
        self.build(queue, list, (target.width, target.height));
        let draws = self.steps.len();
        self.encode(device, queue, target);
        Ok(FrameStats {
            commands: list.len(),
            vertices: self.flat.vertices.len()
                + self.image_vertices.len()
                + self.shadow_vertices.len()
                + self.text_vertices.len(),
            draws,
            cpu_time: started.elapsed(),
        })
    }

    fn build(&mut self, queue: &wgpu::Queue, list: &DisplayList, size: (u32, u32)) {
        let Self {
            steps,
            flat,
            image_vertices,
            image_indices,
            shadow_vertices,
            shadow_indices,
            text_vertices,
            text_indices,
            tessellator,
            text,
            textures,
            ..
        } = self;
        steps.clear();
        flat.vertices.clear();
        flat.indices.clear();
        image_vertices.clear();
        image_indices.clear();
        shadow_vertices.clear();
        shadow_indices.clear();
        text_vertices.clear();
        text_indices.clear();
        text.begin_frame();
        let mut frame = FrameBuild {
            queue,
            steps,
            flat,
            image_vertices,
            image_indices,
            shadow_vertices,
            shadow_indices,
            text_vertices,
            text_indices,
            tessellator,
            text,
            textures,
            clip_depth: 0,
            clip_stack: Vec::new(),
        };
        for command in list {
            frame.command(command);
        }
        drop(frame);
        self.apply_ndc_transform(size);
    }

    /// Viewport transform: staged vertex positions are physical px (y down);
    /// clip space is NDC (y up). Applied once per frame to every family -
    /// the shaders pass positions through unchanged.
    fn apply_ndc_transform(&mut self, size: (u32, u32)) {
        #[allow(clippy::cast_precision_loss)]
        let width = size.0.max(1) as f32;
        #[allow(clippy::cast_precision_loss)]
        let height = size.1.max(1) as f32;
        let transform = |x: &mut f32, y: &mut f32| {
            *x = 2.0 * *x / width - 1.0;
            *y = 1.0 - 2.0 * *y / height;
        };
        for vertex in &mut self.flat.vertices {
            let [x, y, ..] = vertex;
            transform(x, y);
        }
        for vertex in &mut self.image_vertices {
            let [x, y, ..] = vertex;
            transform(x, y);
        }
        for vertex in &mut self.shadow_vertices {
            let [x, y, ..] = vertex;
            transform(x, y);
        }
        for vertex in &mut self.text_vertices {
            let [x, y, ..] = vertex;
            transform(x, y);
        }
    }

    fn encode(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, target: &RenderTarget<'_>) {
        let Self {
            steps,
            buffers,
            flat,
            image_vertices,
            image_indices,
            shadow_vertices,
            shadow_indices,
            text_vertices,
            text_indices,
            vector,
            shadow,
            text,
            textures,
            target: frame_target,
            ..
        } = self;
        let data = StagingData {
            flat_vertices: &flat.vertices,
            flat_indices: &flat.indices,
            image_vertices,
            image_indices,
            shadow_vertices,
            shadow_indices,
            text_vertices,
            text_indices,
        };
        let uploaded = buffers.upload(device, queue, &data);
        let Some(frame) = frame_target.as_ref() else {
            return;
        };
        let pipelines = PipelineRefs {
            vector,
            shadow,
            text,
            textures,
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("render-frame-encoder"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("render-frame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &frame.msaa_view,
                    resolve_target: Some(target.view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &frame.stencil_view,
                    depth_ops: None,
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0),
                        store: wgpu::StoreOp::Discard,
                    }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            for step in steps.iter() {
                draw_step(&mut pass, step, &uploaded, &pipelines);
            }
        }
        queue.submit(Some(encoder.finish()));
    }
}
