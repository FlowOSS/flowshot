//! Per-window surface lifecycle: configuration, resize, and frame
//! presentation for the live overlay shell. The configuration
//! policy itself lives in [`crate::gpu::configure_overlay_surface`] so the
//! `render_smoke` example and the runtime share one implementation.

use std::sync::Arc;
use std::time::Instant;

use winit::window::Window;

use crate::adapter::surface_size_fits;
use crate::crosshair::CrosshairPipeline;
use crate::error::UiError;
use crate::gpu::GpuContext;
use crate::render::{DisplayList, FrameStats, Presenter, RenderTarget, Renderer, SoftwareRenderer};

/// Everything one window's surface needs beyond the shared GPU objects:
/// which monitor it covers (error context), its initial extent, the
/// token-derived crosshair color, and the window handle (the present-time
/// `pre_present_notify` frame-callback seam).
#[derive(Debug, Clone)]
pub(crate) struct SurfaceSpec {
    pub monitor: String,
    pub initial_size: (u32, u32),
    pub crosshair_color: [f32; 4],
    pub window: Arc<Window>,
}

/// One window's configured surface plus its crosshair pipeline.
#[derive(Debug)]
pub(crate) struct WindowSurface {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    crosshair: CrosshairPipeline,
    monitor: String,
    window: Arc<Window>,
}

impl WindowSurface {
    /// Configures `surface` for the transparent overlay and builds its
    /// crosshair pipeline.
    ///
    /// # Errors
    ///
    /// Returns [`UiError::SurfaceSizeExceedsLimits`] when the initial extent
    /// does not fit the device's texture limits (checked BEFORE `configure`,
    /// which panics on oversized extents) and [`UiError::NoSurfaceFormats`]
    /// when the surface advertises no texture formats.
    pub(crate) fn new(
        surface: wgpu::Surface<'static>,
        gpu: &GpuContext,
        spec: &SurfaceSpec,
    ) -> Result<Self, UiError> {
        let config = crate::gpu::configure_overlay_surface(
            &surface,
            &gpu.adapter,
            &gpu.device,
            spec.initial_size.0,
            spec.initial_size.1,
            &spec.monitor,
        )?;
        let format = config.format;
        let crosshair = CrosshairPipeline::new(&gpu.device, format, spec.crosshair_color);
        Ok(Self {
            surface,
            config,
            crosshair,
            monitor: spec.monitor.clone(),
            window: spec.window.clone(),
        })
    }

    /// The configured surface extent in physical pixels.
    pub(crate) const fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    /// The configured surface texture format (the window renderer's target
    /// format).
    pub(crate) const fn format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    /// Reconfigures the surface after a window resize. Zero extents are
    /// ignored (minimized windows); the next valid resize reconfigures.
    ///
    /// # Errors
    ///
    /// Returns [`UiError::SurfaceSizeExceedsLimits`] when the new extent does
    /// not fit the device's texture limits (checked BEFORE `configure`, which
    /// panics on oversized extents); v1 is all-or-nothing, so the caller
    /// tears the whole overlay down with this typed error.
    pub(crate) fn resize(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> Result<(), UiError> {
        if width == 0 || height == 0 {
            return Ok(());
        }
        let limit = device.limits().max_texture_dimension_2d;
        if !surface_size_fits(width, height, limit) {
            return Err(UiError::SurfaceSizeExceedsLimits {
                monitor: self.monitor.clone(),
                width,
                height,
                max: limit,
            });
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(device, &self.config);
        Ok(())
    }

    /// Renders one frame: the optional backdrop/content display list through
    /// the window's [`Renderer`], then the crosshair pass on top. Without
    /// content the frame is a transparent clear plus the crosshair (the
    /// empty-overlay behavior). Returns the content renderer's [`FrameStats`]
    /// (zero stats for an empty frame or a skipped transient-invalid frame),
    /// which the `perf-trace` profiler reads; production discards it.
    ///
    /// # Errors
    ///
    /// Returns the renderer's [`UiError::RenderTargetTooLarge`] for invalid
    /// extents; transient `Lost`/`Outdated`/`Timeout`/`Occluded` states
    /// recover in-place and are reported as `Ok`.
    pub(crate) fn render(
        &self,
        gpu: &GpuContext,
        content: Option<(&mut Renderer, &DisplayList)>,
        vertices: Option<[[f32; 2]; 4]>,
    ) -> Result<FrameStats, UiError> {
        let acquire_started = Instant::now();
        let frame = match self.surface.get_current_texture() {
            // A suboptimal texture still presents; the next acquire reports
            // `Outdated` when a reconfigure is actually due.
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            // Both transient invalidations recover by reconfiguring; the frame
            // is skipped and the next RedrawRequested presents again.
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&gpu.device, &self.config);
                return Ok(FrameStats::default());
            }
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Validation => return Ok(FrameStats::default()),
        };
        let acquire_time = acquire_started.elapsed();
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let has_content = content.is_some();
        let mut stats = match content {
            Some((renderer, list)) => {
                let target = RenderTarget {
                    view: &view,
                    width: self.config.width,
                    height: self.config.height,
                };
                renderer.render(&gpu.device, &gpu.queue, &target, list)?
            }
            None => FrameStats::default(),
        };
        stats.acquire_time = acquire_time;
        if let Some(vertices) = vertices {
            self.crosshair.write_vertices(&gpu.queue, vertices);
        }
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("overlay-frame-encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("overlay-frame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // The content pass already cleared and resolved into
                        // the view; the crosshair pass must preserve it.
                        load: if has_content {
                            wgpu::LoadOp::Load
                        } else {
                            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if vertices.is_some() {
                self.crosshair.draw(&mut pass);
            }
        }
        gpu.queue.submit(Some(encoder.finish()));
        // Notify winit AFTER drawing, BEFORE the present (its documented
        // contract): on Wayland this schedules the frame callback that
        // throttles `RedrawRequested` to the output refresh. Without it the
        // non-blocking `Mailbox` acquire would let motion-driven redraws spin
        // ungated (measured 3500 renders/sec); with it, redraws pace to vsync
        // while input still dispatches at full rate. A no-op on other
        // platforms (the base `Window` API, not a platform extension).
        self.window.pre_present_notify();
        gpu.queue.present(frame);
        Ok(stats)
    }

    /// Presents one CPU-rendered frame: the software path's framebuffer is
    /// uploaded and drawn as one fullscreen quad, then the crosshair pass runs
    /// on top. Same surface lifecycle and same transient-invalidation recovery
    /// as [`WindowSurface::render`]; the difference is only the content pass,
    /// which costs one `write_texture` plus one three-vertex draw.
    pub(crate) fn render_software(
        &mut self,
        gpu: &GpuContext,
        presenter: &mut Presenter,
        renderer: &mut SoftwareRenderer,
        list: &DisplayList,
        vertices: Option<[[f32; 2]; 4]>,
    ) -> FrameStats {
        // Step 1: Render on CPU FIRST before acquiring the surface texture,
        // so the compositor swapchain buffer is never held locked during CPU work.
        renderer.resize(self.config.width, self.config.height);
        let stats = renderer.render(list);

        // Step 2: Acquire the swapchain texture now that pixels are ready to upload.
        let acquire_started = Instant::now();
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&gpu.device, &self.config);
                return stats;
            }
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Validation => return stats,
        };
        let acquire_time = acquire_started.elapsed();
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        // Step 3: Single unified render pass and single queue submit for
        // both the fullscreen software presenter sample and the crosshair overlay.
        let (width, height) = renderer.size();
        let was_resized = width != presenter.size().0 || height != presenter.size().1;
        presenter.ensure(&gpu.device, width, height);
        if was_resized {
            presenter.upload(&gpu.queue, renderer.pixels());
        } else if let Some((origin, extent)) = renderer.last_damage() {
            presenter.upload_region(&gpu.queue, renderer.pixels(), origin, extent);
        } else {
            presenter.upload(&gpu.queue, renderer.pixels());
        }

        if let Some(vertices) = vertices {
            self.crosshair.write_vertices(&gpu.queue, vertices);
        }

        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("overlay-software-frame-encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("overlay-software-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
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
            presenter.draw(&mut pass);
            if vertices.is_some() {
                self.crosshair.draw(&mut pass);
            }
        }
        gpu.queue.submit(Some(encoder.finish()));
        self.window.pre_present_notify();
        gpu.queue.present(frame);
        FrameStats {
            acquire_time,
            ..stats
        }
    }
}
