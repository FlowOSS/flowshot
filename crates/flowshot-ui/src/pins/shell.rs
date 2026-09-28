//! The pin winit shell: event translation, effect application, and window
//! lifecycle (plan todo 30). The pure decisions live in
//! [`super::state::PinState`]; this layer only translates
//! [`WindowEvent`](winit::event::WindowEvent)s into [`PinInput`]s and
//! applies the returned [`PinEffect`]s to real windows.
//!
//! Resize mechanics: `SetWindowSize` pins `min == max == target` inner
//! size - the ONLY client resize path Hyprland honors (it marks every
//! toplevel stateful via unconditional TILED states, so winit's
//! `request_inner_size` is a no-op; live-probed 2026-09-26, see
//! [`super::zoom`]). The compositor answers with `Resized`, which
//! reconfigures the surface and repaints.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::PhysicalKey;
use winit::window::{Window, WindowId};

use super::event::PinInput;
use super::image::PinImage;
use super::runtime::{PinSpec, PinUiEvent, WindowCustomizer};
use super::sink::{PinActionSink, PinId};
use super::spec::{WHEEL_UNITS_PER_PX, WHEEL_UNITS_PER_STEP};
use super::state::{PinBehavior, PinState};
use crate::error::UiError;
use crate::gpu::GpuContext;
use crate::render::Renderer;
use crate::surface::WindowSurface;

/// One pin window and everything the shell needs to drive it.
#[derive(Debug)]
pub(crate) struct PinEntry {
    pub id: PinId,
    pub window: Arc<Window>,
    pub surface: WindowSurface,
    pub renderer: Renderer,
    pub state: PinState,
    pub image: PinImage,
}

/// The application state of the pin event loop.
#[derive(Debug)]
pub(crate) struct PinApp {
    pub specs: Vec<PinSpec>,
    pub entries: Vec<PinEntry>,
    pub window_index: HashMap<WindowId, usize>,
    pub gpu: Option<GpuContext>,
    pub sink: Option<Arc<dyn PinActionSink>>,
    pub behavior: PinBehavior,
    pub customizer: Option<WindowCustomizer>,
    pub fatal_error: Option<UiError>,
}

impl PinApp {
    pub(crate) fn new(
        specs: Vec<PinSpec>,
        sink: Option<Arc<dyn PinActionSink>>,
        behavior: PinBehavior,
    ) -> Self {
        Self {
            specs,
            entries: Vec::new(),
            window_index: HashMap::new(),
            gpu: None,
            sink,
            behavior,
            customizer: None,
            fatal_error: None,
        }
    }

    pub(crate) fn take_fatal_error(&mut self) -> Option<UiError> {
        self.fatal_error.take()
    }

    fn fail(&mut self, target: &ActiveEventLoop, error: UiError) {
        tracing::error!(%error, "pin host initialization failed; exiting");
        self.fatal_error = Some(error);
        target.exit();
    }

    #[cfg(feature = "test-drive")]
    fn entry_for(&self, id: PinId) -> Option<usize> {
        self.entries.iter().position(|entry| entry.id == id)
    }

    /// Routes one input through the pin's state machine and applies the
    /// effects (shared by real events and `test-drive` injections - the
    /// todo-13 single-seam rule).
    pub(crate) fn route(&mut self, target: &ActiveEventLoop, index: usize, input: &PinInput) {
        let Some((effects, id)) = self
            .entries
            .get_mut(index)
            .map(|entry| (entry.state.on_input(input, Instant::now()), entry.id))
        else {
            return;
        };
        self.apply(target, index, id, &effects);
    }
}

impl ApplicationHandler<PinUiEvent> for PinApp {
    fn resumed(&mut self, target: &ActiveEventLoop) {
        if !self.entries.is_empty() || self.fatal_error.is_some() {
            return;
        }
        if let Err(error) = super::spawn::spawn_all(self, target) {
            self.fail(target, error);
        }
    }

    fn window_event(&mut self, target: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
        let Some(index) = self.window_index.get(&window_id).copied() else {
            return;
        };
        let Some(id) = self.entries.get(index).map(|entry| entry.id) else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => {
                self.route(target, index, &PinInput::CloseRequested);
            }
            WindowEvent::Destroyed => self.close(target, index, id),
            WindowEvent::CursorMoved { position, .. } => self.route(
                target,
                index,
                &PinInput::CursorMoved {
                    x: position.x,
                    y: position.y,
                },
            ),
            WindowEvent::CursorEntered { .. } => {
                self.route(target, index, &PinInput::CursorEntered);
            }
            WindowEvent::CursorLeft { .. } => self.route(target, index, &PinInput::CursorLeft),
            WindowEvent::MouseWheel { delta, .. } => {
                let units = match delta {
                    MouseScrollDelta::LineDelta(_, lines) => {
                        f64::from(lines) * WHEEL_UNITS_PER_STEP
                    }
                    MouseScrollDelta::PixelDelta(position) => position.y * WHEEL_UNITS_PER_PX,
                };
                self.route(target, index, &PinInput::Wheel { units });
            }
            WindowEvent::MouseInput { state, button, .. } => self.route(
                target,
                index,
                &PinInput::Button {
                    button,
                    pressed: state == ElementState::Pressed,
                },
            ),
            WindowEvent::KeyboardInput { event: key, .. } => {
                let PhysicalKey::Code(code) = key.physical_key else {
                    return;
                };
                self.route(
                    target,
                    index,
                    &PinInput::Key {
                        code,
                        pressed: key.state == ElementState::Pressed,
                        repeat: key.repeat,
                    },
                );
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.route(target, index, &PinInput::Modifiers(modifiers.state()));
            }
            WindowEvent::Touch(touch) => self.route(
                target,
                index,
                &PinInput::Touch {
                    id: touch.id,
                    phase: touch.phase,
                    x: touch.location.x,
                    y: touch.location.y,
                },
            ),
            WindowEvent::Resized(size) => {
                if let Some(gpu) = self.gpu.as_ref()
                    && let Some(entry) = self.entries.get_mut(index)
                    && let Err(error) = entry.surface.resize(&gpu.device, size.width, size.height)
                {
                    // Unlike the all-or-nothing overlay, one pin's surface
                    // failure never tears down its siblings: log, keep the
                    // last good frame.
                    tracing::error!(%error, pin = id.raw(), "pin surface resize failed");
                    return;
                }
                self.route(
                    target,
                    index,
                    &PinInput::Resized {
                        width: size.width,
                        height: size.height,
                    },
                );
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => self.route(
                target,
                index,
                &PinInput::ScaleFactorChanged { scale_factor },
            ),
            WindowEvent::RedrawRequested => {
                if let Some(gpu) = self.gpu.as_ref()
                    && let Some(entry) = self.entries.get_mut(index)
                {
                    super::paint::render_pin(gpu, entry);
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, target: &ActiveEventLoop) {
        // Zoom-transition frames (todo 41): paced redraws while any pin's
        // zoom eases, capped at its settle deadline; idle pins keep the
        // loop in ControlFlow::Wait (zero CPU).
        let now = Instant::now();
        let mut wake: Option<Instant> = None;
        for entry in &self.entries {
            if !entry.state.zoom_anim_active(now) {
                continue;
            }
            entry.window.request_redraw();
            let paced = now
                .checked_add(crate::motion::FRAME_INTERVAL)
                .unwrap_or(now);
            let next = entry
                .state
                .zoom_anim_deadline()
                .map_or(paced, |deadline| deadline.min(paced));
            wake = Some(wake.map_or(next, |earliest| earliest.min(next)));
        }
        match wake {
            Some(deadline) if deadline > now => {
                target.set_control_flow(ControlFlow::WaitUntil(deadline));
            }
            _ => target.set_control_flow(ControlFlow::Wait),
        }
    }

    fn user_event(&mut self, target: &ActiveEventLoop, event: PinUiEvent) {
        match event {
            PinUiEvent::Exit => {
                tracing::info!("exit requested via pin handle");
                target.exit();
            }
            #[cfg(feature = "test-drive")]
            PinUiEvent::Synthetic { id, input } => {
                if let Some(index) = self.entry_for(id) {
                    self.route(target, index, &input);
                }
            }
        }
    }

    fn exiting(&mut self, _target: &ActiveEventLoop) {
        tracing::info!(pins = self.entries.len(), "pin host shutting down");
    }
}
