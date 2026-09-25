//! Public runtime entry point: event-loop ownership and cross-thread control.
//!
//! [`OverlayRuntime::new`] verifies the session and builds the winit event
//! loop; [`OverlayRuntime::run`] spawns the per-monitor windows on `Resumed`
//! and blocks until teardown. [`OverlayHandle`] controls the running loop
//! from other threads (graceful exit; with feature `test-drive`, synthetic
//! input injection).

use winit::event_loop::{EventLoop, EventLoopProxy};

use crate::app::OverlayApp;
use crate::error::UiError;
#[cfg(feature = "test-drive")]
use crate::input::SyntheticInput;

/// Events sent into the running loop from other threads.
#[derive(Debug, Clone)]
pub(crate) enum UiEvent {
    /// Gracefully close all overlay windows.
    Exit,
    /// Synthetic input injected through the `test-drive` seam.
    #[cfg(feature = "test-drive")]
    Synthetic(SyntheticInput),
}

/// Thread-safe handle to a running overlay.
#[derive(Debug, Clone)]
pub struct OverlayHandle {
    proxy: EventLoopProxy<UiEvent>,
}

impl OverlayHandle {
    /// Requests graceful teardown of all overlay windows.
    ///
    /// # Errors
    ///
    /// Returns [`UiError::EventLoopClosed`] when the loop already exited.
    pub fn request_exit(&self) -> Result<(), UiError> {
        self.proxy
            .send_event(UiEvent::Exit)
            .map_err(|_| UiError::EventLoopClosed)
    }

    /// TEST SEAM (feature `test-drive`, plan todo 13): injects a synthetic
    /// input event into the running loop, routed exactly like a real one -
    /// mouse paths become QA-able without external injection tools.
    ///
    /// # Errors
    ///
    /// Returns [`UiError::EventLoopClosed`] when the loop already exited.
    #[cfg(feature = "test-drive")]
    pub fn inject_event(&self, input: SyntheticInput) -> Result<(), UiError> {
        self.proxy
            .send_event(UiEvent::Synthetic(input))
            .map_err(|_| UiError::EventLoopClosed)
    }
}

/// The multi-monitor overlay runtime.
///
/// # Example (startup shape)
///
/// ```no_run
/// fn main() -> Result<(), flowshot_ui::UiError> {
///     let runtime = flowshot_ui::OverlayRuntime::new()?;
///     let handle = runtime.handle().clone();
///     std::thread::spawn(move || {
///         let _ = handle.request_exit();
///     });
///     runtime.run()
/// }
/// ```
#[derive(Debug)]
pub struct OverlayRuntime {
    event_loop: EventLoop<UiEvent>,
    handle: OverlayHandle,
    app: OverlayApp,
}

impl OverlayRuntime {
    /// Creates the runtime: verifies a display-server session, then builds
    /// the event loop. Windows and GPU objects are created on `Resumed`
    /// inside [`Self::run`].
    ///
    /// # Errors
    ///
    /// Returns [`UiError::NoDisplayServer`] when `WAYLAND_DISPLAY` is unset
    /// or empty (with a hint - `FlowShot` never falls back to X11), and
    /// [`UiError::EventLoop`] when the event loop cannot be created.
    pub fn new() -> Result<Self, UiError> {
        require_display_server()?;
        let event_loop = EventLoop::<UiEvent>::with_user_event().build()?;
        let handle = OverlayHandle {
            proxy: event_loop.create_proxy(),
        };
        Ok(Self {
            event_loop,
            handle,
            app: OverlayApp::new(),
        })
    }

    /// The cross-thread control handle (exit; `test-drive` injection).
    #[must_use]
    pub const fn handle(&self) -> &OverlayHandle {
        &self.handle
    }

    /// Runs the event loop until teardown; consumes the runtime.
    ///
    /// # Errors
    ///
    /// Returns the typed fatal error when startup failed inside the loop
    /// (monitors, windows, GPU adapter/limits) or a runtime surface operation
    /// failed (oversized resize), and [`UiError::EventLoop`] when the loop
    /// itself errored. A clean Esc teardown returns `Ok`.
    pub fn run(self) -> Result<(), UiError> {
        let Self {
            event_loop,
            mut app,
            ..
        } = self;
        let outcome = event_loop.run_app(&mut app);
        if let Some(error) = app.take_fatal_error() {
            return Err(error);
        }
        outcome?;
        Ok(())
    }
}

/// Verifies a compositor session is present before touching winit.
///
/// Portable environment probe (purity gate: no platform imports in this
/// crate): `WAYLAND_DISPLAY` is the standard session variable every Wayland
/// compositor exports to its clients. `FlowShot` never falls back to X11
/// (draft F9), so an unset variable is a typed startup error with a hint -
/// never a silent platform switch. The X11 roadmap phase revisits this probe.
fn require_display_server() -> Result<(), UiError> {
    match std::env::var_os("WAYLAND_DISPLAY") {
        Some(value) if !value.is_empty() => Ok(()),
        _ => Err(UiError::NoDisplayServer),
    }
}
