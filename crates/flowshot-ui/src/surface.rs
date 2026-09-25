//! Per-window surface lifecycle: configuration, resize, and frame
//! presentation for the live overlay (todo 13 shell). The configuration
//! policy itself lives in [`crate::gpu::configure_overlay_surface`] so the
//! `render_smoke` example and the runtime share one implementation.

use crate::adapter::surface_size_fits;
use crate::crosshair::CrosshairPipeline;
use crate::error::UiError;
use crate::gpu::GpuContext;
use crate::render::{DisplayList, RenderTarget, Renderer};

/// Everything one window's surface needs beyond the shared GPU objects:
/// which monitor it covers (error context), its initial extent, and the
/// token-derived crosshair color.
#[derive(Debug, Clone)]
pub(crate) struct SurfaceSpec {
    pub monitor: String,
    pub initial_size: (u32, u32),
    pub crosshair_color: [f32; 4],
}

/// One window's configured surface plus its crosshair pipeline.
#[derive(Debug)]
pub(crate) struct WindowSurface {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    crosshair: CrosshairPipeline,
    monitor: String,
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
    /// todo-13 empty-overlay behavior).
    ///
    /// # Errors
    ///
    /// Returns [`UiError::OutOfMemory`] when presentation exhausts memory and
    /// the renderer's [`UiError::RenderTargetTooLarge`] for invalid extents;
    /// transient `Lost`/`Outdated`/`Timeout` states recover in-place and are
    /// reported as `Ok`.
    pub(crate) fn render(
        &self,
        gpu: &GpuContext,
        content: Option<(&mut Renderer, &DisplayList)>,
        vertices: Option<[[f32; 2]; 4]>,
    ) -> Result<(), UiError> {
        let frame = match self.surface.get_current_texture() {
            Ok(frame) => frame,
            // Both transient invalidations recover by reconfiguring; the frame
            // is skipped and the next RedrawRequested presents again.
            Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
                self.surface.configure(&gpu.device, &self.config);
                return Ok(());
            }
            Err(wgpu::SurfaceError::Timeout) => return Ok(()),
            Err(wgpu::SurfaceError::OutOfMemory) => return Err(UiError::OutOfMemory),
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let has_content = content.is_some();
        if let Some((renderer, list)) = content {
            let target = RenderTarget {
                view: &view,
                width: self.config.width,
                height: self.config.height,
            };
            renderer.render(&gpu.device, &gpu.queue, &target, list)?;
        }
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
            });
            if vertices.is_some() {
                self.crosshair.draw(&mut pass);
            }
        }
        gpu.queue.submit(Some(encoder.finish()));
        frame.present();
        Ok(())
    }
}
