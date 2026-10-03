//! Event-loop ownership and cross-thread control for the consent dialog
//! (the `LauncherWindow` shape): [`ConsentWindow::new`] verifies the
//! session; [`ConsentWindow::run`] spawns the window on `Resumed` and
//! blocks until any dismissal path records the choice.

use winit::event_loop::{EventLoop, EventLoopProxy};

use crate::error::UiError;
use crate::runtime::require_display_server;

use super::app::ConsentApp;
use super::model::ConsentModel;
use super::options::ConsentWindowOptions;

/// Events sent into the running loop from other threads.
#[derive(Debug, Clone)]
pub enum ConsentEvent {
    /// Gracefully close the dialog WITHOUT recording an answer (the
    /// programmatic path - only a user dismissal writes the config, so an
    /// operator-initiated close re-arms the daemon-startup prompt).
    Exit,
}

/// Thread-safe handle to a running consent dialog.
#[derive(Debug, Clone)]
pub struct ConsentHandle {
    proxy: EventLoopProxy<ConsentEvent>,
}

impl ConsentHandle {
    /// Requests a graceful close (no answer recorded).
    ///
    /// # Errors
    ///
    /// Returns [`UiError::EventLoopClosed`] when the loop already exited.
    pub fn request_exit(&self) -> Result<(), UiError> {
        self.proxy
            .send_event(ConsentEvent::Exit)
            .map_err(|_| UiError::EventLoopClosed)
    }
}

/// The consent dialog runtime.
///
/// # Example (startup shape)
///
/// ```no_run
/// fn main() -> Result<(), flowshot_ui::UiError> {
///     let options = flowshot_ui::consent::ConsentWindowOptions::default();
///     flowshot_ui::consent::ConsentWindow::new(options)?.run()
/// }
/// ```
#[derive(Debug)]
pub struct ConsentWindow {
    event_loop: EventLoop<ConsentEvent>,
    handle: ConsentHandle,
    app: ConsentApp,
}

impl ConsentWindow {
    /// Creates the runtime: verifies a display-server session and builds
    /// the event loop. Window and GPU objects are created on `Resumed`
    /// inside [`Self::run`]. The model starts from the recorded defaults
    /// (telemetry checked, technical details unchecked).
    ///
    /// # Errors
    ///
    /// Returns [`UiError::NoDisplayServer`] when `WAYLAND_DISPLAY` is unset
    /// or empty, and [`UiError::EventLoop`] when the event loop cannot be
    /// created.
    pub fn new(options: ConsentWindowOptions) -> Result<Self, UiError> {
        require_display_server()?;
        let event_loop = EventLoop::<ConsentEvent>::with_user_event().build()?;
        let handle = ConsentHandle {
            proxy: event_loop.create_proxy(),
        };
        Ok(Self {
            event_loop,
            handle,
            app: ConsentApp::new(ConsentModel::default(), options),
        })
    }

    /// The cross-thread control handle.
    #[must_use]
    pub const fn handle(&self) -> &ConsentHandle {
        &self.handle
    }

    /// Runs the event loop until the dialog closes; consumes the runtime.
    ///
    /// # Errors
    ///
    /// Returns the typed fatal error when startup failed inside the loop
    /// (window/GPU/surface creation) and [`UiError::EventLoop`] when the
    /// loop itself errored. Every clean close (answer recorded OR
    /// programmatic exit) returns `Ok`.
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
