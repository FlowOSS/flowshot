//! Event-loop ownership and cross-thread control for the launcher dialog
//! (the `SettingsWindow` shape): [`LauncherWindow::new`] verifies the
//! session and runs the live output probe; [`LauncherWindow::run`] spawns
//! the window on `Resumed` and blocks until Capture dispatches or the
//! dialog is cancelled.

use winit::event_loop::{EventLoop, EventLoopProxy};

use crate::error::UiError;
use crate::runtime::require_display_server;

use super::app::LauncherApp;
use super::model::LauncherModel;
use super::options::LauncherWindowOptions;

/// Events sent into the running loop from other threads.
#[derive(Debug, Clone)]
pub enum LauncherEvent {
    /// Gracefully close the window (no dispatch).
    Exit,
    /// Synthetic input injected through the `test-drive` seam.
    #[cfg(feature = "test-drive")]
    Synthetic(super::window::LauncherInput),
}

/// One synthetic input event for the `test-drive` seam (the pins/overlay
/// injector pattern): converted into [`egui::Event`]s and queued on the
/// production [`crate::egui_host::input::InputState`], so keyboard and
/// pointer QA needs no external injection tools (wtype/ydotool are absent
/// on the QA machine, issues.md 2026-09-25). The winit -> egui conversion
/// layer it enters below is the settings keymap's independently tested
/// territory.
#[cfg(feature = "test-drive")]
#[derive(Debug, Clone, PartialEq)]
pub enum LauncherInput {
    /// Printable text committed into the focused field (one key event
    /// carrying the whole string, egui's paste path).
    Text(String),
    /// A named key press + release (Tab / Enter / Escape / arrows).
    Key(winit::keyboard::NamedKey),
    /// A left-button pointer transition at a window-local position in
    /// physical px (the `PinInput` convention): move + button as one event
    /// pair, so a click is `pressed: true` followed by `pressed: false`.
    Pointer {
        /// Window-local horizontal position, physical px.
        x: f64,
        /// Window-local vertical position, physical px.
        y: f64,
        /// Button transition.
        pressed: bool,
    },
}

#[cfg(feature = "test-drive")]
impl LauncherInput {
    pub(super) fn egui_events(&self, pixels_per_point: f32) -> Vec<egui::Event> {
        use egui::{Event, Modifiers, PointerButton, Pos2};
        match self {
            Self::Text(text) => vec![Event::Text(text.clone())],
            Self::Key(named) => {
                let mapped = crate::egui_host::keymap::egui_key_from_logical(
                    &winit::keyboard::Key::Named(*named),
                );
                mapped.map_or_else(Vec::new, |key| {
                    vec![key_event(key, true), key_event(key, false)]
                })
            }
            Self::Pointer { x, y, pressed } => {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "f64 physical px -> f32 egui points; dialog extents are far within f32 range"
                )]
                let pos = Pos2::new(
                    (x / f64::from(pixels_per_point)) as f32,
                    (y / f64::from(pixels_per_point)) as f32,
                );
                vec![
                    Event::PointerMoved(pos),
                    Event::PointerButton {
                        pos,
                        button: PointerButton::Primary,
                        pressed: *pressed,
                        modifiers: Modifiers::NONE,
                    },
                ]
            }
        }
    }
}

#[cfg(feature = "test-drive")]
fn key_event(key: egui::Key, pressed: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

/// Thread-safe handle to a running launcher dialog.
#[derive(Debug, Clone)]
pub struct LauncherHandle {
    proxy: EventLoopProxy<LauncherEvent>,
}

impl LauncherHandle {
    /// Requests a graceful close (no dispatch).
    ///
    /// # Errors
    ///
    /// Returns [`UiError::EventLoopClosed`] when the loop already exited.
    pub fn request_exit(&self) -> Result<(), UiError> {
        self.proxy
            .send_event(LauncherEvent::Exit)
            .map_err(|_| UiError::EventLoopClosed)
    }

    /// TEST SEAM (feature `test-drive`): injects synthetic input into the
    /// dialog's exact production input path.
    ///
    /// # Errors
    ///
    /// Returns [`UiError::EventLoopClosed`] when the loop already exited.
    #[cfg(feature = "test-drive")]
    pub fn inject_event(&self, input: LauncherInput) -> Result<(), UiError> {
        self.proxy
            .send_event(LauncherEvent::Synthetic(input))
            .map_err(|_| UiError::EventLoopClosed)
    }
}

/// The launcher dialog runtime.
///
/// # Example (startup shape)
///
/// ```no_run
/// fn main() -> Result<(), flowshot_ui::UiError> {
///     let options = flowshot_ui::launcher::LauncherWindowOptions::default();
///     flowshot_ui::launcher::LauncherWindow::new(options)?.run()
/// }
/// ```
#[derive(Debug)]
pub struct LauncherWindow {
    event_loop: EventLoop<LauncherEvent>,
    handle: LauncherHandle,
    app: LauncherApp,
}

impl LauncherWindow {
    /// Creates the runtime: verifies a display-server session, runs the
    /// live output probe (once - the dropdown's monitor list), and builds
    /// the event loop. Window and GPU objects are created on `Resumed`
    /// inside [`Self::run`].
    ///
    /// # Errors
    ///
    /// Returns [`UiError::NoDisplayServer`] when `WAYLAND_DISPLAY` is unset
    /// or empty, and [`UiError::EventLoop`] when the event loop cannot be
    /// created.
    pub fn new(options: LauncherWindowOptions) -> Result<Self, UiError> {
        require_display_server()?;
        let event_loop = EventLoop::<LauncherEvent>::with_user_event().build()?;
        let handle = LauncherHandle {
            proxy: event_loop.create_proxy(),
        };
        let outputs = options
            .monitor_probe
            .as_ref()
            .map_or_else(Vec::new, super::options::MonitorProbe::probe);
        Ok(Self {
            event_loop,
            handle,
            app: LauncherApp::new(LauncherModel::from_outputs(outputs), options),
        })
    }

    /// The model before [`Self::run`] (seeding edits, inspecting state).
    pub fn model_mut(&mut self) -> &mut LauncherModel {
        &mut self.app.model
    }

    /// The cross-thread control handle.
    #[must_use]
    pub const fn handle(&self) -> &LauncherHandle {
        &self.handle
    }

    /// Runs the event loop until the dialog closes; consumes the runtime.
    ///
    /// # Errors
    ///
    /// Returns the typed fatal error when startup failed inside the loop
    /// (window/GPU/surface creation) and [`UiError::EventLoop`] when the
    /// loop itself errored. A clean close (Capture dispatched OR cancelled)
    /// returns `Ok`.
    pub fn run(self) -> Result<(), UiError> {
        let Self {
            event_loop,
            mut app,
            ..
        } = self;
        let outcome = event_loop.run_app(&mut app);
        if let Some(error) = app.fatal.take() {
            return Err(error);
        }
        outcome?;
        Ok(())
    }
}
