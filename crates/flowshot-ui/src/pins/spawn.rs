//! Pin window + GPU spawn.
//!
//! Window policy: borderless (`with_decorations(false)`), transparent,
//! always-on-top BEST EFFORT (`WindowLevel::AlwaysOnTop` is advisory on
//! Wayland - xdg-shell has no stacking protocol; Hyprland's `staysontop`
//! window rule ships in the Wayland setup snippet), initial extent = the
//! state machine's clamped target, and min == max pinned to that extent so
//! the compositor cannot user-resize the pin out of sync with its image.
//! NO layer-shell (the v1 overlay policy's Must-NOT).
//!
//! The clamp screen is the primary monitor's physical size, additionally
//! capped by the device's `max_texture_dimension_2d` (the downlevel-limits
//! lesson, issues.md 2026-09-25: surfaces are validated BEFORE configure,
//! which panics on oversized extents).

use std::sync::Arc;

use winit::dpi::PhysicalSize;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowAttributes, WindowLevel};

use super::runtime::PinSpec;
use super::shell::{PinApp, PinEntry};
use super::state::PinState;
use super::strings::WINDOW_TITLE;
use crate::crosshair::parse_srgb_hex;
use crate::error::UiError;
use crate::gpu::GpuContext;
use crate::render::{Renderer, RgbaImage};
use crate::surface::{SurfaceSpec, WindowSurface};

pub(super) fn spawn_all(app: &mut PinApp, target: &ActiveEventLoop) -> Result<(), UiError> {
    let (screen, scale_factor) = spawn_screen(target);
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: crate::gpu::OVERLAY_BACKENDS,
        ..wgpu::InstanceDescriptor::default()
    });
    let mut windows = Vec::with_capacity(app.specs.len());
    for spec in app.specs.drain(..) {
        let state = PinState::new(
            (spec.image.width, spec.image.height),
            screen,
            scale_factor,
            app.behavior.clone(),
        );
        let (width, height) = state.target_window();
        tracing::info!(
            pin = spec.id.raw(),
            image = ?(spec.image.width, spec.image.height),
            screen = ?screen,
            scale = state.scale(),
            window = ?(width, height),
            "pin window planned"
        );
        let attributes = pin_window_attributes(width, height);
        let attributes = match &app.customizer {
            Some(customizer) => customizer.apply(attributes),
            None => attributes,
        };
        let window = Arc::new(target.create_window(attributes).map_err(|source| {
            UiError::WindowCreation {
                monitor: format!("pin-{}", spec.id.raw()),
                source,
            }
        })?);
        // The pin IS the content; the compositor cursor stays visible
        // (unlike the capture overlay's self-drawn crosshair).
        window.request_redraw();
        let surface =
            instance
                .create_surface(window.clone())
                .map_err(|source| UiError::SurfaceCreation {
                    monitor: format!("pin-{}", spec.id.raw()),
                    source,
                })?;
        windows.push((spec, state, window, surface));
    }
    let probe = windows
        .first()
        .map(|(_, _, _, surface)| surface)
        .ok_or(UiError::NoPinsRequested)?;
    let gpu = GpuContext::new(&instance, probe)?;
    let limit = gpu.device.limits().max_texture_dimension_2d;
    let accent = parse_srgb_hex(&app.behavior.tokens.palette.accent)
        .unwrap_or(crate::crosshair::FALLBACK_COLOR);
    for (spec, state, window, surface) in windows {
        let monitor = format!("pin-{}", spec.id.raw());
        let (width, height) = state.target_window();
        let surface_spec = SurfaceSpec {
            monitor,
            initial_size: (width.min(limit), height.min(limit)),
            crosshair_color: accent,
            window: window.clone(),
        };
        let window_surface = WindowSurface::new(surface, &gpu, &surface_spec)?;
        let mut renderer = Renderer::new(&gpu.device, &gpu.queue, window_surface.format());
        upload_texture(&gpu, &mut renderer, &spec, &state)?;
        let id = spec.id;
        app.window_index.insert(window.id(), app.entries.len());
        app.entries.push(PinEntry {
            id,
            window,
            surface: window_surface,
            renderer,
            state,
            image: spec.image,
        });
    }
    app.gpu = Some(gpu);
    tracing::info!(pins = app.entries.len(), "pin windows spawned");
    Ok(())
}

/// The clamp screen (primary monitor physical size, texture-limit capped)
/// and its scale factor.
fn spawn_screen(target: &ActiveEventLoop) -> ((u32, u32), f64) {
    let monitor = target
        .primary_monitor()
        .or_else(|| target.available_monitors().next());
    match monitor {
        Some(monitor) => {
            let size = monitor.size();
            ((size.width, size.height), monitor.scale_factor())
        }
        // No monitor report: fall back to a 1080p-class clamp; window
        // creation will surface the real failure typed.
        None => ((1920, 1080), 1.0),
    }
}

fn pin_window_attributes(width: u32, height: u32) -> WindowAttributes {
    let size = PhysicalSize::new(width, height);
    Window::default_attributes()
        .with_title(WINDOW_TITLE)
        .with_inner_size(size)
        .with_min_inner_size(size)
        .with_max_inner_size(size)
        .with_transparent(true)
        .with_decorations(false)
        .with_resizable(false)
        .with_window_level(WindowLevel::AlwaysOnTop)
}

fn upload_texture(
    gpu: &GpuContext,
    renderer: &mut Renderer,
    spec: &PinSpec,
    state: &PinState,
) -> Result<(), UiError> {
    let buffer = spec.image.composed(state.rotation(), state.opacity())?;
    renderer.textures_mut().insert(
        &gpu.device,
        &gpu.queue,
        super::TEXTURE_ID,
        &RgbaImage {
            width: spec.image.width,
            height: spec.image.height,
            data: &buffer,
        },
    )
}
