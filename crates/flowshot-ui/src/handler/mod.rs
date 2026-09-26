//! Window spawn and GPU surface initialization.
//!
//! Spawn policy (plan todo 13): one borderless-fullscreen window per monitor,
//! transparent, undecorated, always-on-top best effort (advisory on Wayland -
//! compositor policy decides; the stale `with_always_on_top` builder does not
//! exist in winit 0.30, Oracle r1 #2). Event dispatch lives in
//! [`super::handler::events`]; every event funnels through
//! [`OverlayCore::route`](crate::OverlayCore), the central input router.

mod events;

use std::sync::Arc;

use winit::event_loop::ActiveEventLoop;
use winit::monitor::MonitorHandle;
use winit::window::{Fullscreen, Window, WindowAttributes, WindowLevel};

use crate::app::{OverlayApp, WindowEntry};
use crate::error::UiError;
use crate::gpu::GpuContext;
use crate::monitor;
use crate::render::Renderer;
use crate::router::WindowSlot;
use crate::surface::{SurfaceSpec, WindowSurface};

impl OverlayApp {
    pub(crate) fn spawn(&mut self, target: &ActiveEventLoop) -> Result<(), UiError> {
        let monitors: Vec<MonitorHandle> = target.available_monitors().collect();
        // The capture-provided layout supersedes the monitor-derived one
        // (plan todo 15): true transforms and scales from the capture pass.
        let (layout, bindings) = if let Some(backdrop) = self.backdrop.as_ref() {
            monitor::bindings_for_layout(backdrop.layout(), &monitors)
        } else {
            let layout = monitor::layout_from_monitors(&monitors)?;
            let bindings: Vec<usize> = (0..layout.outputs.len()).collect();
            (layout, bindings)
        };
        self.core.router_mut().install(layout, bindings);
        for (index, handle) in monitors.iter().enumerate() {
            let slot = WindowSlot::new(index);
            let monitor_name = monitor::monitor_name(handle, index);
            let window = Arc::new(
                target
                    .create_window(overlay_window_attributes(handle))
                    .map_err(|source| UiError::WindowCreation {
                        monitor: monitor_name.clone(),
                        source,
                    })?,
            );
            // The crosshair is drawn by us; the compositor cursor would be
            // invisible over the fullscreen surface anyway (#1659-class fix).
            window.set_cursor_visible(false);
            // Always-on IME model (draft D7): enabled from map, never toggled
            // per text session.
            window.set_ime_allowed(true);
            window.request_redraw();
            self.window_index.insert(window.id(), slot);
            self.windows.push(WindowEntry {
                window,
                surface: None,
                monitor_name,
                renderer: None,
                effect_textures: Vec::new(),
            });
        }
        self.init_surfaces()?;
        if let Some(backdrop) = self.backdrop.as_ref() {
            for (connector, reason) in backdrop.missing() {
                tracing::error!(
                    connector,
                    ?reason,
                    "frozen frame missing; window shows the letterbox placeholder"
                );
            }
        }
        tracing::info!(windows = self.windows.len(), "overlay windows spawned");
        Ok(())
    }

    fn init_surfaces(&mut self) -> Result<(), UiError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: crate::gpu::OVERLAY_BACKENDS,
            ..wgpu::InstanceDescriptor::default()
        });
        let mut surfaces = Vec::with_capacity(self.windows.len());
        for entry in &self.windows {
            let surface = instance
                .create_surface(entry.window.clone())
                .map_err(|source| UiError::SurfaceCreation {
                    monitor: entry.monitor_name.clone(),
                    source,
                })?;
            surfaces.push(surface);
        }
        let probe = surfaces.first().ok_or(UiError::NoMonitors)?;
        let gpu = GpuContext::new(&instance, probe)?;
        for (entry, surface) in self.windows.iter_mut().zip(surfaces) {
            let size = entry.window.inner_size();
            let spec = SurfaceSpec {
                monitor: entry.monitor_name.clone(),
                initial_size: (size.width, size.height),
                crosshair_color: self.crosshair_color,
            };
            entry.surface = Some(WindowSurface::new(surface, &gpu, &spec)?);
        }
        self.init_renderers(&gpu)?;
        self.gpu = Some(gpu);
        Ok(())
    }

    /// Builds each window's renderer and uploads the frozen-frame textures
    /// (plan todo 15). The renderer exists even without a backdrop - the
    /// selection visuals (todo 16) render through it on the empty overlay
    /// too. Upload failure is fatal and typed - the overlay never presents a
    /// silent black frame.
    fn init_renderers(&mut self, gpu: &GpuContext) -> Result<(), UiError> {
        let Self {
            windows,
            backdrop,
            core,
            ..
        } = self;
        for (index, entry) in windows.iter_mut().enumerate() {
            let Some(surface) = entry.surface.as_ref() else {
                continue;
            };
            let mut renderer = Renderer::new(&gpu.device, &gpu.queue, surface.format());

            let atlas_image = crate::render::RgbaImage {
                width: crate::widgets::ATLAS_WIDTH,
                height: crate::widgets::ATLAS_HEIGHT,
                data: crate::widgets::ICON_ATLAS,
            };
            renderer.textures_mut().insert(
                &gpu.device,
                &gpu.queue,
                crate::widgets::ICON_ATLAS_ID,
                &atlas_image,
            )?;

            if let Some(backdrop) = backdrop.as_mut() {
                let slot = WindowSlot::new(index);
                if let Some(output_index) = core.router().output_index_for(slot) {
                    backdrop.upload_for(output_index, &mut renderer, gpu)?;
                }
                backdrop.upload_cursor(&mut renderer, gpu)?;
            }
            entry.renderer = Some(renderer);
        }
        Ok(())
    }
}

fn overlay_window_attributes(monitor: &MonitorHandle) -> WindowAttributes {
    Window::default_attributes()
        .with_title("FlowShot")
        .with_fullscreen(Some(Fullscreen::Borderless(Some(monitor.clone()))))
        .with_transparent(true)
        .with_decorations(false)
        .with_window_level(WindowLevel::AlwaysOnTop)
}
