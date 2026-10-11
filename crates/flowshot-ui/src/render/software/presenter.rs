//! Presents a CPU-rendered frame: one RGBA8 texture plus one fullscreen-triangle
//! sample, the whole GPU side of the software path.
//!
//! The GPU renderer's per-frame GPU work is a multisampled offscreen target, a
//! stencil attachment, and a multi-pass submit. This presenter's per-frame GPU
//! work is one `write_texture` and one three-vertex draw, so a desktop whose
//! driver cannot afford the former can still afford the latter.
//!
//! # Sampling contract
//!
//! The vertex stage emits `uv = (x, -y) * 0.5 + 0.5`, which at a fragment's
//! pixel center is exactly the texel center of the same screen coordinate — a
//! 1:1 copy with no resampling and no frame-extent uniform. The framebuffer is
//! filled top-row-first (physical px, y down) while clip space is y up, so the
//! v axis is flipped. The source texture is always `RGBA8Unorm` (the
//! framebuffer's own premultiplied linear-light space) regardless of the
//! surface format, and the blend is premultiplied: the sRGB target's store
//! re-encodes, which is what puts the CPU path in the GPU path's byte space.

use std::borrow::Cow;

use crate::render::software::SoftwareRenderer;

/// The software path's present shader: the fullscreen triangle is synthesized
/// from `vertex_index`, so the draw needs no vertex buffer at all.
const PRESENT_WGSL: &str = r"
struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(0) @binding(0) var frame_texture: texture_2d<f32>;
@group(0) @binding(1) var frame_sampler: sampler;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VsOut {
    let x = select(-1.0, 3.0, (index & 1u) != 0u);
    let y = select(-1.0, 3.0, (index & 2u) != 0u);
    var out: VsOut;
    out.clip = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>(x, -y) * 0.5 + vec2<f32>(0.5, 0.5);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(frame_texture, frame_sampler, in.uv);
}";

/// The synthesized fullscreen triangle's vertex count.
const VERTEX_COUNT: u32 = 3;

/// One window's software presenter: the RGBA8 source texture, the three-vertex
/// draw, and the bind group that ties them together.
#[derive(Debug)]
pub struct Presenter {
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    layout: wgpu::BindGroupLayout,
    texture: Option<wgpu::Texture>,
    view: Option<wgpu::TextureView>,
    bind_group: Option<wgpu::BindGroup>,
    width: u32,
    height: u32,
}

impl Presenter {
    /// A presenter with no source texture yet; [`Presenter::ensure`] creates one
    /// at the frame extent.
    #[must_use]
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("software-present.wgsl"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(PRESENT_WGSL)),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("software-present-layout"),
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
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("software-present-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("software-present"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..wgpu::PrimitiveState::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
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
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("software-present-sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..wgpu::SamplerDescriptor::default()
        });
        Self {
            pipeline,
            sampler,
            layout,
            texture: None,
            view: None,
            bind_group: None,
            width: 0,
            height: 0,
        }
    }

    /// (Re)creates the source texture when the frame extent changes, and
    /// rebuilds the bind group that references it.
    pub fn ensure(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        if width == self.width && height == self.height {
            return;
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("software-frame-source"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("software-frame-bind-group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        self.texture = Some(texture);
        self.view = Some(view);
        self.bind_group = Some(bind_group);
        self.width = width;
        self.height = height;
    }

    /// Copies one CPU frame into the source texture.
    pub fn upload(&mut self, queue: &wgpu::Queue, bytes: &[u8]) {
        let Some(texture) = self.texture.as_ref() else {
            return;
        };
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: 0, y: 0, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(self.width * 4),
                rows_per_image: Some(self.height),
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Copies a damaged sub-region of the CPU frame into the source texture.
    pub fn upload_region(
        &mut self,
        queue: &wgpu::Queue,
        bytes: &[u8],
        origin: (u32, u32),
        extent: (u32, u32),
    ) {
        let Some(texture) = self.texture.as_ref() else {
            return;
        };
        let (origin_x, origin_y) = origin;
        let (width, height) = extent;
        if width == 0
            || height == 0
            || origin_x + width > self.width
            || origin_y + height > self.height
        {
            return;
        }
        let start = (origin_y as usize * self.width as usize + origin_x as usize) * 4;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: origin_x,
                    y: origin_y,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            &bytes[start..],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(self.width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    }

    /// The source texture extent.
    #[must_use]
    pub const fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Draws the fullscreen sample into `pass`.
    pub fn draw<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>) {
        pass.set_pipeline(&self.pipeline);
        if let Some(bind_group) = self.bind_group.as_ref() {
            pass.set_bind_group(0, Some(bind_group), &[]);
        }
        pass.draw(0..VERTEX_COUNT, 0..1);
    }

    /// Presents one CPU frame: the texture upload plus the fullscreen draw.
    /// A frame whose extent never went through [`Presenter::ensure`] draws
    /// nothing (the caller skipped the texture) and reports zero stats.
    pub fn present(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view: &wgpu::TextureView,
        renderer: &SoftwareRenderer,
    ) {
        let (width, height) = renderer.size();
        let was_resized = width != self.width || height != self.height;
        self.ensure(device, width, height);
        if was_resized {
            self.upload(queue, renderer.pixels());
        } else if let Some((origin, extent)) = renderer.last_damage() {
            self.upload_region(queue, renderer.pixels(), origin, extent);
        } else {
            self.upload(queue, renderer.pixels());
        }
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("software-present-encoder"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("software-present-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.draw(&mut pass);
        }
        queue.submit(Some(encoder.finish()));
    }
}
