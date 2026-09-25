//! Overlay application state and frame rendering.
//!
//! Frames are rendered only in response to `RedrawRequested` (input, resize,
//! spawn); the dispatch half lives in [`crate::handler`] and deliberately
//! idles in `about_to_wait`, so the default `ControlFlow::Wait` keeps an idle
//! overlay at zero CPU (plan todo 13).

use std::collections::HashMap;
use std::sync::Arc;

use flowshot_core::geometry::OutputLayout;
use flowshot_core::tokens::DesignTokens;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

use crate::crosshair;
use crate::error::UiError;
use crate::gpu::{GpuContext, WindowSurface};
use crate::input::Action;
use crate::router::{InputRouter, WindowSlot};
use crate::state::OverlayCore;

/// One per-monitor overlay window and its GPU surface.
#[derive(Debug)]
pub(crate) struct WindowEntry {
    pub window: Arc<Window>,
    pub surface: Option<WindowSurface>,
    pub monitor_name: String,
}

/// The application state driven by the winit event loop.
#[derive(Debug)]
pub(crate) struct OverlayApp {
    pub core: OverlayCore,
    pub windows: Vec<WindowEntry>,
    pub window_index: HashMap<WindowId, WindowSlot>,
    pub gpu: Option<GpuContext>,
    /// Set when a fatal typed error (startup OR a runtime surface failure)
    /// must abort the loop; [`OverlayRuntime::run`](crate::OverlayRuntime)
    /// returns it after teardown so the process exits 1, never panics.
    pub fatal_error: Option<UiError>,
    pub crosshair_color: [f32; 4],
}

impl OverlayApp {
    pub(crate) fn new() -> Self {
        let tokens = DesignTokens::default();
        let crosshair_color =
            crosshair::parse_srgb_hex(&tokens.palette.accent).unwrap_or(crosshair::FALLBACK_COLOR);
        Self {
            core: OverlayCore::new(InputRouter::new(OutputLayout::new(Vec::new()), Vec::new())),
            windows: Vec::new(),
            window_index: HashMap::new(),
            gpu: None,
            fatal_error: None,
            crosshair_color,
        }
    }

    pub(crate) fn take_fatal_error(&mut self) -> Option<UiError> {
        self.fatal_error.take()
    }

    pub(crate) fn apply_actions(&mut self, target: &ActiveEventLoop, actions: &[Action]) {
        for action in actions {
            match *action {
                Action::Redraw(slot) => self.request_redraw(slot),
                Action::Exit => {
                    // Esc on any window closes all: one shared session.
                    tracing::info!("exit requested via input; closing all overlay windows");
                    target.exit();
                }
            }
        }
    }

    pub(crate) fn request_redraw(&self, slot: WindowSlot) {
        if let Some(entry) = self.windows.get(slot.index()) {
            entry.window.request_redraw();
        }
    }

    pub(crate) fn render_window(&mut self, slot: WindowSlot) {
        let _span = tracing::trace_span!("frame.render", window = slot.index()).entered();
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        let Some(entry) = self.windows.get_mut(slot.index()) else {
            return;
        };
        let Some(surface) = entry.surface.as_mut() else {
            return;
        };
        // The crosshair follows the window that last received motion; during
        // an implicit grab that is the drag-origin window even while the
        // cursor is logically over a neighbor.
        let vertices = self
            .core
            .cursor()
            .filter(|cursor| cursor.slot == slot)
            .and_then(|cursor| {
                let (local_x, local_y) = self.core.router().to_local(slot, cursor.clamped)?;
                let (width, height) = surface.size();
                crosshair::crosshair_vertices(local_x, local_y, f64::from(width), f64::from(height))
            });
        if let Err(error) = surface.render(gpu, vertices) {
            tracing::error!(%error, window = slot.index(), "frame presentation failed");
        }
    }
}
