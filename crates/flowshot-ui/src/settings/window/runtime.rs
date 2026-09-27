//! Event-loop ownership and cross-thread control (the `OverlayRuntime`
//! shape): [`SettingsWindow::new`] verifies the session and loads the model;
//! [`SettingsWindow::run`] spawns the window on `Resumed` and blocks until
//! close.

use std::path::Path;

use winit::event_loop::{EventLoop, EventLoopProxy};

use crate::error::UiError;
use crate::runtime::require_display_server;

use super::super::model::SettingsModel;
use super::app::SettingsApp;
use super::options::SettingsWindowOptions;
use crate::egui_host::theme::ThemeMode;

/// Events sent into the running loop from other threads.
#[derive(Debug, Clone, Copy)]
pub enum SettingsEvent {
    /// Gracefully close the window.
    Exit,
    /// The system dark/light preference changed (portal notification).
    SetSystemTheme(ThemeMode),
}

/// Thread-safe handle to a running settings window.
#[derive(Debug, Clone)]
pub struct SettingsHandle {
    proxy: EventLoopProxy<SettingsEvent>,
}

impl SettingsHandle {
    /// Requests a graceful close.
    ///
    /// # Errors
    ///
    /// Returns [`UiError::EventLoopClosed`] when the loop already exited.
    pub fn request_exit(&self) -> Result<(), UiError> {
        self.proxy
            .send_event(SettingsEvent::Exit)
            .map_err(|_| UiError::EventLoopClosed)
    }

    /// Pushes a new system theme preference (live portal follow).
    ///
    /// # Errors
    ///
    /// Returns [`UiError::EventLoopClosed`] when the loop already exited.
    pub fn set_system_theme(&self, mode: ThemeMode) -> Result<(), UiError> {
        self.proxy
            .send_event(SettingsEvent::SetSystemTheme(mode))
            .map_err(|_| UiError::EventLoopClosed)
    }
}

/// The settings window runtime.
///
/// # Example (startup shape)
///
/// ```no_run
/// fn main() -> Result<(), flowshot_ui::UiError> {
///     let options = flowshot_ui::settings::SettingsWindowOptions::new(std::path::PathBuf::from(
///         "/tmp/flowshot.toml",
///     ));
///     flowshot_ui::settings::SettingsWindow::new(options)?.run()
/// }
/// ```
#[derive(Debug)]
pub struct SettingsWindow {
    event_loop: EventLoop<SettingsEvent>,
    handle: SettingsHandle,
    app: SettingsApp,
}

impl SettingsWindow {
    /// Creates the runtime: verifies a display-server session, loads the
    /// model from `options.config_path` (missing file = defaults; corrupt
    /// file = defaults + banner), and builds the event loop. Window and GPU
    /// objects are created on `Resumed` inside [`Self::run`].
    ///
    /// # Errors
    ///
    /// Returns [`UiError::NoDisplayServer`] when `WAYLAND_DISPLAY` is unset
    /// or empty, and [`UiError::EventLoop`] when the event loop cannot be
    /// created.
    pub fn new(options: SettingsWindowOptions) -> Result<Self, UiError> {
        require_display_server()?;
        let event_loop = EventLoop::<SettingsEvent>::with_user_event().build()?;
        let handle = SettingsHandle {
            proxy: event_loop.create_proxy(),
        };
        let model = load_model(&options.config_path);
        Ok(Self {
            event_loop,
            handle,
            app: SettingsApp::new(model, options),
        })
    }

    /// The model before [`Self::run`] (seeding edits, inspecting state).
    pub fn model_mut(&mut self) -> &mut SettingsModel {
        &mut self.app.model
    }

    /// The cross-thread control handle.
    #[must_use]
    pub const fn handle(&self) -> &SettingsHandle {
        &self.handle
    }

    /// Runs the event loop until the window closes; consumes the runtime.
    ///
    /// # Errors
    ///
    /// Returns the typed fatal error when startup failed inside the loop
    /// (window/GPU/surface creation) and [`UiError::EventLoop`] when the
    /// loop itself errored. A clean close returns `Ok`.
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

/// The resilient load rule (CLI/daemon precedent): a missing or unreadable
/// file yields defaults with a warning - never a failure; a corrupt but
/// readable file yields defaults + the repair banner.
fn load_model(path: &Path) -> SettingsModel {
    match std::fs::read_to_string(path) {
        Ok(text) => SettingsModel::from_toml_str(&text),
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "settings load failed; using defaults");
            SettingsModel::default()
        }
    }
}
