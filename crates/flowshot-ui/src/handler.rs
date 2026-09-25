//! Window spawn and winit event dispatch (`ApplicationHandler`).
//!
//! Spawn policy (plan todo 13): one borderless-fullscreen window per monitor,
//! transparent, undecorated, always-on-top best effort (advisory on Wayland -
//! compositor policy decides; the stale `with_always_on_top` builder does not
//! exist in winit 0.30, Oracle r1 #2). Every event funnels through
//! [`OverlayCore::route`](crate::OverlayCore), the central input router.

use std::sync::Arc;

use flowshot_core::geometry;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::PhysicalKey;
use winit::monitor::MonitorHandle;
use winit::window::{Fullscreen, Window, WindowAttributes, WindowId, WindowLevel};

use crate::app::{OverlayApp, WindowEntry};
use crate::error::UiError;
use crate::gpu::GpuContext;
use crate::input::InputEvent;
use crate::monitor;
use crate::router::WindowSlot;
use crate::runtime::UiEvent;
use crate::surface::{SurfaceSpec, WindowSurface};

impl OverlayApp {
    pub(crate) fn spawn(&mut self, target: &ActiveEventLoop) -> Result<(), UiError> {
        let monitors: Vec<MonitorHandle> = target.available_monitors().collect();
        let layout = monitor::layout_from_monitors(&monitors)?;
        let bindings: Vec<usize> = (0..layout.outputs.len()).collect();
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
            });
        }
        self.init_surfaces()?;
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
        self.gpu = Some(gpu);
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

impl ApplicationHandler<UiEvent> for OverlayApp {
    fn resumed(&mut self, target: &ActiveEventLoop) {
        if !self.windows.is_empty() || self.fatal_error.is_some() {
            return;
        }
        if let Err(error) = self.spawn(target) {
            tracing::error!(%error, "overlay initialization failed; exiting");
            self.fatal_error = Some(error);
            target.exit();
        }
    }

    fn window_event(&mut self, target: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
        let Some(slot) = self.window_index.get(&window_id).copied() else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => {
                tracing::debug!(
                    window = slot.index(),
                    "close requested; tearing down all windows"
                );
                target.exit();
            }
            WindowEvent::Destroyed => {
                self.window_index.remove(&window_id);
                if let Some(entry) = self.windows.get_mut(slot.index()) {
                    entry.surface = None;
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let report = self.core.route(
                    slot,
                    &InputEvent::PointerMoved {
                        x: position.x,
                        y: position.y,
                    },
                );
                self.apply_actions(target, &report.actions);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let report = self.core.route(
                    slot,
                    &InputEvent::PointerButton {
                        button,
                        pressed: state == ElementState::Pressed,
                    },
                );
                self.apply_actions(target, &report.actions);
            }
            WindowEvent::KeyboardInput { event: key, .. } => {
                let PhysicalKey::Code(code) = key.physical_key else {
                    return;
                };
                let report = self.core.route(
                    slot,
                    &InputEvent::Key {
                        code,
                        pressed: key.state == ElementState::Pressed,
                        repeat: key.repeat,
                    },
                );
                self.apply_actions(target, &report.actions);
            }
            WindowEvent::Ime(ime) => {
                let report = self.core.route(slot, &InputEvent::Ime(ime));
                self.apply_actions(target, &report.actions);
            }
            WindowEvent::Resized(size) => {
                let mut resize_failure = None;
                if let Some(gpu) = self.gpu.as_ref()
                    && let Some(entry) = self.windows.get_mut(slot.index())
                    && let Some(surface) = entry.surface.as_mut()
                    && let Err(error) = surface.resize(&gpu.device, size.width, size.height)
                {
                    resize_failure = Some(error);
                }
                if let Some(error) = resize_failure {
                    // v1 all-or-nothing: one monitor's surface failure tears
                    // the whole overlay down with a typed error (exit 1),
                    // never a wgpu configure panic.
                    tracing::error!(%error, window = slot.index(), "surface resize failed; tearing down");
                    self.fatal_error = Some(error);
                    target.exit();
                    return;
                }
                // Refine the router geometry from the real surface extent
                // (post-transform; corrects rotated outputs, see monitor.rs).
                let buffer = geometry::PhysicalSize::from_raw(
                    i32::try_from(size.width).unwrap_or(i32::MAX),
                    i32::try_from(size.height).unwrap_or(i32::MAX),
                );
                if let Err(error) = self.core.router_mut().update_surface_size(slot, buffer) {
                    tracing::error!(%error, window = slot.index(), "surface size update rejected");
                }
                self.request_redraw(slot);
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                if let Err(error) = self.core.router_mut().update_scale(slot, scale_factor) {
                    tracing::error!(%error, window = slot.index(), scale_factor, "scale update rejected");
                }
                // winit follows with Resized carrying the new physical extent.
            }
            WindowEvent::RedrawRequested => self.render_window(slot),
            WindowEvent::Focused(focused) => {
                tracing::trace!(window = slot.index(), focused, "focus changed");
            }
            // Remaining variants (touch, wheel, modifiers, theme, file drops)
            // are irrelevant at this stage; wheel/modifiers arrive with the
            // tool layer (todo 20).
            _ => {}
        }
    }

    fn user_event(&mut self, target: &ActiveEventLoop, event: UiEvent) {
        match event {
            UiEvent::Exit => {
                tracing::info!("exit requested via overlay handle");
                target.exit();
            }
            #[cfg(feature = "test-drive")]
            UiEvent::Synthetic(synthetic) => {
                let report = self.core.route(synthetic.slot, &synthetic.event);
                self.apply_actions(target, &report.actions);
            }
        }
    }

    fn exiting(&mut self, _target: &ActiveEventLoop) {
        tracing::info!(windows = self.windows.len(), "overlay shutting down");
    }
}
