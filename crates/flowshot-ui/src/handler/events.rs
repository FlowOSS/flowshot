//! The winit `ApplicationHandler` dispatch (plan todo 13/16).
//!
//! Every window event funnels through [`OverlayCore::route`](crate::OverlayCore),
//! the single input seam real events and `test-drive` injections share.
//! `about_to_wait` owns the HUD hide-time wake: the loop idles in
//! `ControlFlow::Wait` (zero CPU) and switches to `WaitUntil(deadline)` only
//! while a HUD countdown runs, redrawing every window once when it expires.

use std::time::Instant;

use flowshot_core::geometry;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::PhysicalKey;
use winit::window::WindowId;

use crate::app::OverlayApp;
use crate::editor::WHEEL_ANGLE_PER_LINE;
use crate::input::InputEvent;
use crate::router::WindowSlot;
use crate::runtime::UiEvent;

/// Converts a winit scroll delta into Qt-style wheel-angle units (a
/// standard 3-line notch = 120 units, the space the F27
/// `MOUSE_WHEEL_TRESHOLD = 60` constant is defined in). Pixel deltas
/// (touchpads) pass through: their magnitude order matches the angle units
/// (Flameshot's touchpad comment - "value 2 or more, usually 2-8").
#[expect(
    clippy::cast_possible_truncation,
    reason = "wheel angles are clamped into the i32 range before the cast"
)]
fn wheel_angle(delta: MouseScrollDelta) -> i32 {
    let raw = match delta {
        MouseScrollDelta::LineDelta(_, lines) => f64::from(lines) * f64::from(WHEEL_ANGLE_PER_LINE),
        MouseScrollDelta::PixelDelta(position) => position.y,
    };
    let clamped = raw.round().clamp(f64::from(i32::MIN), f64::from(i32::MAX));
    clamped as i32
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
                self.route_sync(
                    target,
                    slot,
                    &InputEvent::PointerButton {
                        button,
                        pressed: state == ElementState::Pressed,
                    },
                );
            }
            WindowEvent::KeyboardInput { event: key, .. } => {
                let PhysicalKey::Code(code) = key.physical_key else {
                    return;
                };
                self.route_sync(
                    target,
                    slot,
                    &InputEvent::Key {
                        code,
                        pressed: key.state == ElementState::Pressed,
                        repeat: key.repeat,
                        text: key.text.as_deref().map(str::to_owned),
                    },
                );
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                let report = self
                    .core
                    .route(slot, &InputEvent::Modifiers(modifiers.state()));
                self.apply_actions(target, &report.actions);
            }
            WindowEvent::Ime(ime) => {
                self.route_sync(target, slot, &InputEvent::Ime(ime));
            }
            WindowEvent::MouseWheel { delta, .. } => self.route_wheel(target, slot, delta),
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
            // Remaining variants (touch, theme, file drops) are irrelevant
            // at this stage.
            _ => {}
        }
    }

    fn about_to_wait(&mut self, target: &ActiveEventLoop) {
        let now = Instant::now();
        if self.core.tick(now) {
            // The HUD hide deadline passed: one redraw clears it everywhere.
            for entry in &self.windows {
                entry.window.request_redraw();
            }
        }
        match self.core.selection().hud_wake() {
            Some(deadline) if deadline > now => {
                target.set_control_flow(ControlFlow::WaitUntil(deadline));
            }
            _ => target.set_control_flow(ControlFlow::Wait),
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

impl OverlayApp {
    /// Routes one input event, applies the shell actions, and mirrors the
    /// text-edit caret into the window's IME cursor area (todo 22 - keys,
    /// pointer buttons, and IME events can all move the caret).
    fn route_sync(&mut self, target: &ActiveEventLoop, slot: WindowSlot, event: &InputEvent) {
        let report = self.core.route(slot, event);
        self.apply_actions(target, &report.actions);
        self.sync_ime_area(slot);
    }

    /// Routes one wheel event as an angle delta (the todo-20 tool-size
    /// adjuster consumes it; zero deltas are dropped).
    fn route_wheel(&mut self, target: &ActiveEventLoop, slot: WindowSlot, delta: MouseScrollDelta) {
        let delta_y = wheel_angle(delta);
        if delta_y != 0 {
            let report = self.core.route(slot, &InputEvent::Wheel { delta_y });
            self.apply_actions(target, &report.actions);
        }
    }
}
