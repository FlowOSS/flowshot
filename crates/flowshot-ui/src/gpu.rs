//! wgpu device management and per-window surface lifecycle.
//!
//! Surface policy (plan todo 13): `Bgra8UnormSrgb` preferred, alpha mode
//! `PreMultiplied` (transparent overlay windows), present mode `Fifo`
//! (vsync, guaranteed-available, zero tearing). Frames are rendered only on
//! `RedrawRequested`, so an idle overlay performs no GPU work at all.
//!
//! Adapter policy (issues.md 2026-09-25 "downlevel wgpu limits"): adapters
//! are enumerated and filtered by the [`crate::adapter`] texture-size floor,
//! and every surface extent is validated against the device limits BEFORE
//! `Surface::configure` (which panics on oversized extents), so GPU failures
//! are typed [`UiError`]s end to end.

use wgpu::{CompositeAlphaMode, PresentMode, TextureFormat};

use crate::adapter::{
    AdapterCandidate, MIN_TEXTURE_DIMENSION_2D, PowerClass, select_adapter, surface_size_fits,
};
use crate::crosshair::CrosshairPipeline;
use crate::error::UiError;

/// Backends for the wgpu instance and adapter enumeration.
///
/// `PRIMARY` (Vulkan on Linux) deliberately excludes the GL/EGL backend: its
/// `eglTerminate` teardown segfaults inside NVIDIA's EGL-Wayland shim
/// (`wl_proxy_marshal_array_flags` on a freed proxy - live core dump
/// 2026-09-25) even when the chosen adapter is Vulkan, because the default
/// `InstanceDescriptor` initializes every backend eagerly. Vulkan covers the
/// plan's environments, including GPU-less ones via Mesa's software
/// rasterizer (lavapipe).
pub(crate) const OVERLAY_BACKENDS: wgpu::Backends = wgpu::Backends::PRIMARY;

/// The process-wide GPU objects shared by every window surface.
///
/// The [`wgpu::Instance`] is not retained: adapter, device, and surfaces each
/// hold their own `Arc` to the shared wgpu context (wgpu 0.20), so dropping
/// the instance after initialization is a no-op for the live objects.
#[derive(Debug)]
pub(crate) struct GpuContext {
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl GpuContext {
    /// Requests adapter and device compatible with `probe_surface`.
    ///
    /// Blocks the calling thread on the one-shot async wgpu handshake; called
    /// exactly once during startup, before the event loop spins. The caller
    /// keeps ownership of `instance` (it may outlive this context; surfaces
    /// and devices hold their own `Arc` to the shared wgpu context).
    ///
    /// # Errors
    ///
    /// Returns [`UiError::NoGpuAdapter`] when no adapter is enumerated,
    /// [`UiError::NoQualifiedAdapter`] (listing every adapter name and its
    /// `max_texture_dimension_2d`) when none meets the texture floor with
    /// surface support, and [`UiError::GpuDevice`] when device creation
    /// fails.
    pub(crate) fn new(
        instance: &wgpu::Instance,
        probe_surface: &wgpu::Surface<'static>,
    ) -> Result<Self, UiError> {
        let (adapter, device, queue) =
            futures::executor::block_on(request_adapter_device(instance, probe_surface))?;
        Ok(Self {
            adapter,
            device,
            queue,
        })
    }
}

async fn request_adapter_device(
    instance: &wgpu::Instance,
    probe_surface: &wgpu::Surface<'static>,
) -> Result<(wgpu::Adapter, wgpu::Device, wgpu::Queue), UiError> {
    let mut adapters = instance.enumerate_adapters(OVERLAY_BACKENDS);
    if adapters.is_empty() {
        return Err(UiError::NoGpuAdapter);
    }
    let mut candidates: Vec<AdapterCandidate> = adapters
        .iter()
        .map(|adapter| {
            let info = adapter.get_info();
            AdapterCandidate {
                name: info.name,
                power: PowerClass::from_device_type(info.device_type),
                surface_supported: adapter.is_surface_supported(probe_surface),
                limits: adapter.limits(),
            }
        })
        .collect();
    let Some(index) = select_adapter(&candidates) else {
        return Err(UiError::NoQualifiedAdapter {
            required: MIN_TEXTURE_DIMENSION_2D,
            adapters: candidates.iter().map(AdapterCandidate::report).collect(),
        });
    };
    // `adapters` and `candidates` are index-aligned; `swap_remove` yields the
    // chosen adapter without an `Option` because the index came from the same
    // length (no unwrap needed).
    let adapter = adapters.swap_remove(index);
    let chosen = candidates.swap_remove(index);
    tracing::info!(
        adapter = %chosen.name,
        max_texture_dimension_2d = chosen.limits.max_texture_dimension_2d,
        "overlay GPU adapter selected"
    );
    let (device, queue) = adapter
        .request_device(
            &wgpu::DeviceDescriptor {
                label: Some("flowshot-overlay-device"),
                required_features: wgpu::Features::empty(),
                // The adapter's own limits: guaranteed to satisfy the 4K
                // texture floor (selection rejected weaker adapters) and
                // never the 2048-capped downlevel defaults.
                required_limits: adapter.limits(),
            },
            None,
        )
        .await?;
    Ok((adapter, device, queue))
}

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
        let (width, height) = (spec.initial_size.0.max(1), spec.initial_size.1.max(1));
        let limit = gpu.device.limits().max_texture_dimension_2d;
        if !surface_size_fits(width, height, limit) {
            return Err(UiError::SurfaceSizeExceedsLimits {
                monitor: spec.monitor.clone(),
                width,
                height,
                max: limit,
            });
        }
        let capabilities = surface.get_capabilities(&gpu.adapter);
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(|format| *format == TextureFormat::Bgra8UnormSrgb)
            .or_else(|| capabilities.formats.first().copied())
            .ok_or_else(|| UiError::NoSurfaceFormats {
                monitor: spec.monitor.clone(),
            })?;
        let alpha_mode = if capabilities
            .alpha_modes
            .contains(&CompositeAlphaMode::PreMultiplied)
        {
            CompositeAlphaMode::PreMultiplied
        } else {
            capabilities
                .alpha_modes
                .first()
                .copied()
                .unwrap_or(CompositeAlphaMode::Opaque)
        };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode: PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: Vec::new(),
        };
        tracing::debug!(
            monitor = %spec.monitor,
            ?format,
            ?alpha_mode,
            width = config.width,
            height = config.height,
            "surface configured"
        );
        surface.configure(&gpu.device, &config);
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

    /// Renders one frame: transparent clear plus the crosshair when
    /// `vertices` is present.
    ///
    /// # Errors
    ///
    /// Returns [`UiError::OutOfMemory`] when presentation exhausts memory;
    /// transient `Lost`/`Outdated`/`Timeout` states recover in-place and are
    /// reported as `Ok`.
    pub(crate) fn render(
        &self,
        gpu: &GpuContext,
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
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
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
