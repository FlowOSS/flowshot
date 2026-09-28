//! wgpu device management and per-window surface lifecycle.
//!
//! Surface policy: `Bgra8UnormSrgb` preferred, alpha mode
//! `PreMultiplied` (transparent overlay windows), present mode `Fifo`
//! (vsync, guaranteed-available, zero tearing). Frames are rendered only on
//! `RedrawRequested`, so an idle overlay performs no GPU work at all.
//!
//! Adapter policy (issues.md 2026-09-25 "downlevel wgpu limits"): adapters
//! are enumerated and filtered by the `crate::adapter::MIN_TEXTURE_DIMENSION_2D` texture-size floor,
//! and every surface extent is validated against the device limits BEFORE
//! `Surface::configure` (which panics on oversized extents), so GPU failures
//! are typed [`UiError`]s end to end.

use wgpu::{CompositeAlphaMode, PresentMode, TextureFormat};

use crate::adapter::{
    AdapterCandidate, MIN_TEXTURE_DIMENSION_2D, PowerClass, select_adapter, surface_size_fits,
};
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
pub const OVERLAY_BACKENDS: wgpu::Backends = wgpu::Backends::PRIMARY;

/// The process-wide GPU objects shared by every window surface.
///
/// The [`wgpu::Instance`] is not retained: adapter, device, and surfaces each
/// hold their own `Arc` to the shared wgpu context (wgpu 0.20), so dropping
/// the instance after initialization is a no-op for the live objects.
#[derive(Debug)]
pub struct GpuContext {
    /// The selected adapter (kept for surface capability queries).
    pub adapter: wgpu::Adapter,
    /// The device, requested with the adapter's own limits (never the
    /// 2048-capped downlevel defaults).
    pub device: wgpu::Device,
    /// The submission queue.
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
    pub fn new(
        instance: &wgpu::Instance,
        probe_surface: &wgpu::Surface<'static>,
    ) -> Result<Self, UiError> {
        let (adapter, device, queue) =
            futures::executor::block_on(request_adapter_device(instance, Some(probe_surface)))?;
        Ok(Self {
            adapter,
            device,
            queue,
        })
    }

    /// Requests adapter and device for **offscreen** rendering (no surface
    /// probe): the cross-rasterizer parity harness and golden fixtures render
    /// to textures headlessly, with the same backend restriction and texture
    /// floor as the live path.
    ///
    /// # Errors
    ///
    /// Same failure modes as [`Self::new`] minus surface support (which is
    /// not probed).
    pub fn new_headless(instance: &wgpu::Instance) -> Result<Self, UiError> {
        let (adapter, device, queue) =
            futures::executor::block_on(request_adapter_device(instance, None))?;
        Ok(Self {
            adapter,
            device,
            queue,
        })
    }
}

async fn request_adapter_device(
    instance: &wgpu::Instance,
    probe_surface: Option<&wgpu::Surface<'static>>,
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
                surface_supported: probe_surface
                    .is_none_or(|surface| adapter.is_surface_supported(surface)),
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
                // Opt-in to adapter-specific format capabilities when the
                // adapter offers them: enables 8x MSAA on formats where the
                // WebGPU core guarantee is only [1, 4] samples (the renderer
                // falls back to 4x without the feature).
                required_features: adapter
                    .features()
                    .intersection(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES),
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

/// Configures `surface` with the overlay policy: `Bgra8UnormSrgb` preferred,
/// premultiplied alpha (transparent overlay windows), `Fifo` present mode
/// (vsync, guaranteed available, zero tearing). Shared by the live overlay
/// windows and the `render_smoke` example so the policy exists exactly once.
///
/// # Errors
///
/// Returns [`UiError::SurfaceSizeExceedsLimits`] when the extent does not fit
/// the device's texture limits (checked BEFORE `configure`, which panics on
/// oversized extents) and [`UiError::NoSurfaceFormats`] when the surface
/// advertises no texture formats.
pub fn configure_overlay_surface(
    surface: &wgpu::Surface<'static>,
    adapter: &wgpu::Adapter,
    device: &wgpu::Device,
    width: u32,
    height: u32,
    monitor: &str,
) -> Result<wgpu::SurfaceConfiguration, UiError> {
    configure_surface(surface, adapter, device, width, height, monitor, true)
}

/// Configures `surface` for an OPAQUE normal window (the settings
/// surface and the launcher dialog): same format/present policy as
/// the overlay, `Opaque` compositing (the window is not transparent).
///
/// # Errors
///
/// Same failure modes as [`configure_overlay_surface`].
pub fn configure_opaque_surface(
    surface: &wgpu::Surface<'static>,
    adapter: &wgpu::Adapter,
    device: &wgpu::Device,
    width: u32,
    height: u32,
    label: &str,
) -> Result<wgpu::SurfaceConfiguration, UiError> {
    configure_surface(surface, adapter, device, width, height, label, false)
}

fn configure_surface(
    surface: &wgpu::Surface<'static>,
    adapter: &wgpu::Adapter,
    device: &wgpu::Device,
    width: u32,
    height: u32,
    monitor: &str,
    transparent: bool,
) -> Result<wgpu::SurfaceConfiguration, UiError> {
    let (width, height) = (width.max(1), height.max(1));
    let limit = device.limits().max_texture_dimension_2d;
    if !surface_size_fits(width, height, limit) {
        return Err(UiError::SurfaceSizeExceedsLimits {
            monitor: monitor.to_owned(),
            width,
            height,
            max: limit,
        });
    }
    let capabilities = surface.get_capabilities(adapter);
    let format = capabilities
        .formats
        .iter()
        .copied()
        .find(|format| *format == TextureFormat::Bgra8UnormSrgb)
        .or_else(|| capabilities.formats.first().copied())
        .ok_or_else(|| UiError::NoSurfaceFormats {
            monitor: monitor.to_owned(),
        })?;
    let preferred_alpha = if transparent {
        CompositeAlphaMode::PreMultiplied
    } else {
        CompositeAlphaMode::Opaque
    };
    let alpha_mode = if capabilities.alpha_modes.contains(&preferred_alpha) {
        preferred_alpha
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
        monitor = %monitor,
        ?format,
        ?alpha_mode,
        width = config.width,
        height = config.height,
        "surface configured"
    );
    surface.configure(device, &config);
    Ok(config)
}
