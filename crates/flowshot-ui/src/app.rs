//! Overlay application state and frame rendering.
//!
//! Frames are rendered only in response to `RedrawRequested` (input, resize,
//! spawn); the dispatch half lives in `crate::handler` and deliberately
//! idles in `about_to_wait`, so the default `ControlFlow::Wait` keeps an idle
//! overlay at zero CPU (plan todo 13).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use flowshot_core::geometry::{LogicalPoint, OutputLayout};
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

use crate::backdrop::{Backdrop, BackdropOptions};
use crate::crosshair;
use crate::editor::ToolCursor;
use crate::error::UiError;
use crate::export::sync_effect_textures;
use crate::gpu::GpuContext;
use crate::input::Action;
use crate::render::{Renderer, RgbaImage, TextureId};
use crate::router::{InputRouter, WindowSlot};
use crate::state::OverlayCore;
use crate::surface::WindowSurface;

/// One per-monitor overlay window and its GPU surface.
#[derive(Debug)]
pub(crate) struct WindowEntry {
    pub window: Arc<Window>,
    pub surface: Option<WindowSurface>,
    pub monitor_name: String,
    /// The window's own renderer (todo 15): holds THIS output's frozen-frame
    /// texture, so per-window redraws never thrash a shared MSAA target.
    pub renderer: Option<Renderer>,
    /// The pixel-effect texture ids currently uploaded to THIS window's
    /// renderer (todo 23: the sync diff base - effects are editor state,
    /// textures are per-renderer).
    pub effect_textures: Vec<TextureId>,
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
    /// The frozen-frame backdrop (plan todo 15); `None` = the empty overlay
    /// (todo-13 behavior: transparent clear + crosshair only).
    pub backdrop: Option<Backdrop>,
    pub backdrop_options: BackdropOptions,
    /// The binary-layer window-attributes hook (Wayland `app_id`; the
    /// `pins::WindowCustomizer` seam - the lib stays platform-pure).
    pub customizer: Option<crate::pins::WindowCustomizer>,
    /// The last IME cursor area sent to a window (todo 22: the caret mirror
    /// for `set_ime_cursor_area`, deduplicated so the compositor is not
    /// spammed on every event).
    pub ime_area: Option<(WindowSlot, [i32; 4])>,
}

impl OverlayApp {
    pub(crate) fn new() -> Self {
        Self {
            core: OverlayCore::new(InputRouter::new(OutputLayout::new(Vec::new()), Vec::new())),
            windows: Vec::new(),
            window_index: HashMap::new(),
            gpu: None,
            fatal_error: None,
            backdrop: None,
            backdrop_options: BackdropOptions::default(),
            customizer: None,
            ime_area: None,
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
                // The funnel already showed the wheel (core-owned chrome
                // state - the headless path owns the whole picker flow);
                // the shell arm only repaints every window.
                Action::ColorWheel => {
                    for entry in &self.windows {
                        entry.window.request_redraw();
                    }
                }
                // Capture-completing gestures: the shell renders the export
                // offscreen and hands it to the installed CompletionSink
                // (todo 38); encoding/actions are the binary layer's.
                Action::Accept
                | Action::Copy
                | Action::Save
                | Action::Pin
                | Action::Upload
                | Action::OpenWith => self.complete(target, *action),
                // The funnel already delivered the pick to the color-pick
                // sink; nothing to repaint (the eyedropper changes no chrome).
                Action::ColorPicked => {}
            }
        }
    }

    pub(crate) fn request_redraw(&self, slot: WindowSlot) {
        if let Some(entry) = self.windows.get(slot.index()) {
            entry.window.request_redraw();
        }
    }

    /// Mirrors the text-edit caret into the window's IME cursor area so the
    /// compositor anchors the IME popup at the caret (todo 22; the iced
    /// `enable_ime` parity - global logical caret rect converted to THIS
    /// window's local physical px, deduplicated per change).
    pub(crate) fn sync_ime_area(&mut self, slot: WindowSlot) {
        let area = self.core.editor().ime_cursor_area().and_then(|rect| {
            let origin = LogicalPoint::from_raw(rect.x.0, rect.y.0);
            let (x, y) = self.core.router().to_local(slot, origin)?;
            let scale = self.core.router().output_for(slot).map_or(1.0, |o| o.scale);
            #[expect(
                clippy::cast_possible_truncation,
                reason = "caret geometry is clamped into the i32 range before the cast"
            )]
            let area = [
                x.clamp(f64::from(i32::MIN), f64::from(i32::MAX)).round() as i32,
                y.clamp(f64::from(i32::MIN), f64::from(i32::MAX)).round() as i32,
                (rect.width.0 * scale).round().max(1.0) as i32,
                (rect.height.0 * scale).round().max(1.0) as i32,
            ];
            Some(area)
        });
        let Some(area) = area else {
            self.ime_area = None;
            return;
        };
        if self.ime_area == Some((slot, area)) {
            return;
        }
        self.ime_area = Some((slot, area));
        if let Some(entry) = self.windows.get(slot.index()) {
            tracing::trace!(
                window = slot.index(),
                area = ?area,
                "ime cursor area"
            );
            entry.window.set_ime_cursor_area(
                winit::dpi::PhysicalPosition::new(area[0], area[1]),
                winit::dpi::PhysicalSize::new(area[2], area[3]),
            );
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
        // cursor is logically over a neighbor. The active tool may suppress
        // it (todo 20 cursor change: the tool's own preview is the cursor).
        let vertices = self
            .core
            .cursor()
            .filter(|cursor| cursor.slot == slot)
            .filter(|_| self.core.editor().cursor_shape() != ToolCursor::Hidden)
            .and_then(|cursor| {
                let (local_x, local_y) = self.core.router().to_local(slot, cursor.clamped)?;
                let (width, height) = surface.size();
                crosshair::crosshair_vertices(local_x, local_y, f64::from(width), f64::from(height))
            });
        // The todo-23 pixel-effect textures must exist in THIS renderer
        // before the display list references them (missing ids draw the
        // magenta placeholder).
        if let Some(renderer) = entry.renderer.as_mut() {
            let effects = self.core.editor().pixel_effects();
            if let Err(error) =
                sync_effect_textures(renderer, gpu, &mut entry.effect_textures, effects)
            {
                tracing::error!(%error, window = slot.index(), "pixel effect texture sync failed");
            }
        }
        // The frame's display list builds through the SHARED builder (todo
        // 41 extraction): the live shell and the offscreen QA harnesses run
        // the identical paint path.
        // The frozen-frame backdrop (todo 15) with the LIVE selection
        // cutout (todo 16: the engine's rect supersedes the construction-
        // time option, so the dim follows the drag).
        let options = BackdropOptions {
            selection: self.core.selection().rect(),
            ..self.backdrop_options
        };
        let frame = crate::frame::build_overlay_frame(
            &self.core,
            slot,
            surface.size(),
            self.backdrop.as_ref().map(|backdrop| (backdrop, &options)),
            Instant::now(),
        );
        let list = frame.list;
        // Magnifier (todo 17): the zoom texture is CPU-built per frame and
        // must be uploaded before the list renders (a missing id draws the
        // magenta placeholder).
        if let (Some(renderer), Some(texture)) = (entry.renderer.as_mut(), frame.magnifier.as_ref())
        {
            let image = RgbaImage {
                width: texture.width,
                height: texture.height,
                data: &texture.pixels,
            };
            if let Err(error) =
                renderer
                    .textures_mut()
                    .insert(&gpu.device, &gpu.queue, texture.id, &image)
            {
                tracing::error!(%error, window = slot.index(), "magnifier texture upload failed");
            }
        }
        let content = match (entry.renderer.as_mut(), list.is_empty()) {
            (Some(renderer), false) => Some((renderer, &list)),
            _ => None,
        };
        if let Err(error) = surface.render(gpu, content, vertices) {
            tracing::error!(%error, window = slot.index(), "frame presentation failed");
        }
    }
}
