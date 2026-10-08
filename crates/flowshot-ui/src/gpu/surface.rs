//! Per-window surface configuration.
//!
//! Surface policy: alpha mode `PreMultiplied` for transparent overlay
//! windows / `Opaque` for the egui host windows, present mode `Fifo`
//! (vsync, guaranteed-available, zero tearing). Frames are rendered only on
//! `RedrawRequested`, so an idle overlay performs no GPU work at all.
//!
//! Format policy is per surface kind (see [`SurfaceKind`]): the overlay
//! keeps its sRGB target; the egui host windows prefer a non-sRGB target
//! because egui does its own gamma handling.

use wgpu::{CompositeAlphaMode, PresentMode, TextureFormat};

use crate::adapter::surface_size_fits;
use crate::error::UiError;

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
    configure_surface(
        surface,
        adapter,
        device,
        width,
        height,
        monitor,
        SurfaceKind::Overlay,
    )
}

/// Configures `surface` for an OPAQUE egui host window (the settings
/// surface, the launcher dialog, and the consent dialog): a non-sRGB format
/// preferred (egui does its own gamma handling), `Opaque` compositing (the
/// window is not transparent), same present-mode policy as the overlay.
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
    configure_surface(
        surface,
        adapter,
        device,
        width,
        height,
        label,
        SurfaceKind::EguiHost,
    )
}

/// Which window a surface configuration belongs to: the format and alpha
/// compositing policies differ per kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SurfaceKind {
    /// The capture overlay (and pins): sRGB content rendered through the
    /// linear-light pipeline, composited transparently by the compositor.
    Overlay,
    /// An opaque egui host window (settings, launcher, consent).
    EguiHost,
}

fn configure_surface(
    surface: &wgpu::Surface<'static>,
    adapter: &wgpu::Adapter,
    device: &wgpu::Device,
    width: u32,
    height: u32,
    monitor: &str,
    kind: SurfaceKind,
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
    let format = select_surface_format(&capabilities.formats, kind, monitor)?;
    let preferred_alpha = match kind {
        SurfaceKind::Overlay => CompositeAlphaMode::PreMultiplied,
        SurfaceKind::EguiHost => CompositeAlphaMode::Opaque,
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
        // `Auto` reproduces wgpu's historical color-space behavior (sRGB for
        // 8-bit targets): the pre-30 API had no color-space choice.
        color_space: wgpu::SurfaceColorSpace::Auto,
        width,
        height,
        present_mode: select_present_mode(&capabilities.present_modes),
        desired_maximum_frame_latency: 2,
        alpha_mode,
        view_formats: Vec::new(),
    };
    tracing::debug!(
        monitor = %monitor,
        ?format,
        ?alpha_mode,
        present_mode = ?config.present_mode,
        width = config.width,
        height = config.height,
        "surface configured"
    );
    surface.configure(device, &config);
    Ok(config)
}

/// Picks the surface texture format for `kind` from the advertised formats.
///
/// The overlay keeps `Bgra8UnormSrgb`: the frozen frame is sRGB content
/// rendered through the linear-light pipeline (the rendering evidence
/// depends on that target). The egui host prefers a NON-sRGB 8-bit target
/// (`Bgra8Unorm`/`Rgba8Unorm`): egui does its own gamma handling, so an
/// sRGB surface renders colors slightly off (and `egui_wgpu` warns); sRGB is
/// the egui host's last resort, logged at debug.
///
/// # Errors
///
/// [`UiError::NoSurfaceFormats`] when the surface advertises no formats.
fn select_surface_format(
    formats: &[TextureFormat],
    kind: SurfaceKind,
    label: &str,
) -> Result<TextureFormat, UiError> {
    let preferred = match kind {
        SurfaceKind::Overlay => [TextureFormat::Bgra8UnormSrgb].as_slice(),
        SurfaceKind::EguiHost => [TextureFormat::Bgra8Unorm, TextureFormat::Rgba8Unorm].as_slice(),
    };
    let format = preferred
        .iter()
        .copied()
        .find(|format| formats.contains(format))
        .or_else(|| formats.first().copied())
        .ok_or_else(|| UiError::NoSurfaceFormats {
            monitor: label.to_owned(),
        })?;
    if kind == SurfaceKind::EguiHost && format.is_srgb() {
        tracing::debug!(
            surface = %label,
            ?format,
            "egui surface fell back to an sRGB format (no non-sRGB target advertised); colors render through egui's linear-framebuffer path"
        );
    }
    Ok(format)
}

/// Picks the surface present mode: the first non-blocking mode the surface
/// advertises, falling back to `Fifo` (the WebGPU-guaranteed mode).
///
/// The overlay renders on the single-threaded winit event loop, so a blocking
/// `get_current_texture` acquire stalls input dispatch for a whole vblank -
/// measured at p50 15.7 ms / p99 38.6 ms on a 60 Hz output under a drag storm
/// (the interactive freeze). `Mailbox` never blocks the acquire (the compositor
/// takes the latest committed buffer; on Wayland it composites atomically, so
/// there is no client-side tear), and winit's frame callbacks already pace
/// redraws to the output refresh, so the non-blocking acquire renders no extra
/// frames. `FifoRelaxed` is the next-best (vsync while in budget, no stall when
/// late); `Fifo` is the guaranteed fallback.
#[must_use]
fn select_present_mode(supported: &[PresentMode]) -> PresentMode {
    const PREFERENCE: [PresentMode; 3] = [
        PresentMode::Mailbox,
        PresentMode::FifoRelaxed,
        PresentMode::Fifo,
    ];
    PREFERENCE
        .into_iter()
        .find(|mode| supported.contains(mode))
        // The WebGPU spec guarantees `Fifo` is always advertised, so the
        // fallback is unreachable; `Fifo` keeps the fn total (never panics).
        .unwrap_or(PresentMode::Fifo)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn overlay_format_keeps_the_srgb_target() {
        // The frozen frame is sRGB content rendered through the linear-light
        // pipeline: the overlay keeps Bgra8UnormSrgb
        // even when the surface advertises non-sRGB formats first.
        let formats = [
            TextureFormat::Bgra8Unorm,
            TextureFormat::Bgra8UnormSrgb,
            TextureFormat::Rgba8UnormSrgb,
        ];
        assert_eq!(
            select_surface_format(&formats, SurfaceKind::Overlay, "test").unwrap(),
            TextureFormat::Bgra8UnormSrgb
        );
        // No sRGB advertised -> the first format (the pre-existing fallback).
        let linear_only = [TextureFormat::Rgba16Float, TextureFormat::Bgra8Unorm];
        assert_eq!(
            select_surface_format(&linear_only, SurfaceKind::Overlay, "test").unwrap(),
            TextureFormat::Rgba16Float
        );
    }

    #[test]
    fn egui_host_format_prefers_non_srgb() {
        // egui does its own gamma handling: on the realistic Wayland
        // advertisement (sRGB listed first) the egui host must pick the
        // non-sRGB 8-bit target - Bgra8UnormSrgb selection is what triggered
        // egui_wgpu's "linear (sRGBA aware) framebuffer" warning.
        let formats = [
            TextureFormat::Bgra8UnormSrgb,
            TextureFormat::Bgra8Unorm,
            TextureFormat::Rgba8UnormSrgb,
            TextureFormat::Rgba8Unorm,
        ];
        assert_eq!(
            select_surface_format(&formats, SurfaceKind::EguiHost, "test").unwrap(),
            TextureFormat::Bgra8Unorm
        );
        let rgba_only = [TextureFormat::Bgra8UnormSrgb, TextureFormat::Rgba8Unorm];
        assert_eq!(
            select_surface_format(&rgba_only, SurfaceKind::EguiHost, "test").unwrap(),
            TextureFormat::Rgba8Unorm
        );
    }

    #[test]
    fn egui_host_format_falls_back_to_srgb_when_nothing_else_exists() {
        // The sRGB fallback is a last resort (debug-logged), not an error.
        let srgb_only = [TextureFormat::Bgra8UnormSrgb, TextureFormat::Rgba8UnormSrgb];
        assert_eq!(
            select_surface_format(&srgb_only, SurfaceKind::EguiHost, "test").unwrap(),
            TextureFormat::Bgra8UnormSrgb
        );
        // No formats at all -> the typed error, for both kinds.
        assert!(matches!(
            select_surface_format(&[], SurfaceKind::EguiHost, "test"),
            Err(UiError::NoSurfaceFormats { .. })
        ));
        assert!(matches!(
            select_surface_format(&[], SurfaceKind::Overlay, "test"),
            Err(UiError::NoSurfaceFormats { .. })
        ));
    }

    #[test]
    fn present_mode_prefers_the_non_blocking_mailbox() {
        // The freeze fix: a surface advertising Mailbox must never fall back to
        // the blocking Fifo acquire (measured 15.7ms p50 on the 60Hz output).
        let supported = [
            PresentMode::Fifo,
            PresentMode::FifoRelaxed,
            PresentMode::Mailbox,
            PresentMode::Immediate,
        ];
        assert_eq!(select_present_mode(&supported), PresentMode::Mailbox);
    }

    #[test]
    fn present_mode_falls_back_to_fifo_relaxed_then_fifo() {
        let relaxed_only = [PresentMode::Fifo, PresentMode::FifoRelaxed];
        assert_eq!(
            select_present_mode(&relaxed_only),
            PresentMode::FifoRelaxed,
            "no Mailbox -> the next non-blocking-capable mode"
        );
        let fifo_only = [PresentMode::Fifo];
        assert_eq!(
            select_present_mode(&fifo_only),
            PresentMode::Fifo,
            "the WebGPU-guaranteed mode is always selectable"
        );
    }

    #[test]
    fn present_mode_skips_unlisted_modes_and_stays_total() {
        // Immediate is not in the preference list (it tears unconditionally and
        // is often unavailable on Wayland), so it is skipped; an empty slice
        // still yields the total Fifo fallback rather than panicking.
        assert_eq!(
            select_present_mode(&[PresentMode::Immediate]),
            PresentMode::Fifo
        );
        assert_eq!(select_present_mode(&[]), PresentMode::Fifo);
    }
}
