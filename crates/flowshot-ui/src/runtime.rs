//! Public runtime entry point: event-loop ownership and cross-thread control.
//!
//! [`OverlayRuntime::new`] verifies the session and builds the winit event
//! loop; [`OverlayRuntime::run`] spawns the per-monitor windows on `Resumed`
//! and blocks until teardown. [`OverlayHandle`] controls the running loop
//! from other threads (graceful exit; with feature `test-drive`, synthetic
//! input injection).

use winit::event_loop::{EventLoop, EventLoopProxy};

use crate::app::OverlayApp;
use crate::backdrop::{Backdrop, BackdropOptions, FrozenCapture};
use crate::error::UiError;
#[cfg(feature = "test-drive")]
use crate::input::SyntheticInput;
use crate::selection::SelectionConfig;
use crate::state::OverlayCore;
use flowshot_core::tokens::DesignTokens;

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

    /// TEST SEAM (feature `test-drive`): injects a synthetic
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
    /// Returns [`UiError::NoDisplayServer`] when neither `WAYLAND_DISPLAY`
    /// nor `DISPLAY` is set (or both are empty), and [`UiError::EventLoop`]
    /// when the event loop cannot be created.
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

    /// Creates the runtime around a frozen capture: every
    /// window renders its output's frozen frame 1:1 as the backdrop, with
    /// the dim layer, selection cutout, and cursor sprite per `options`.
    ///
    /// The capture-provided layout supersedes the winit monitor report
    /// (true transforms and scales); windows bind to outputs by connector
    /// name, then physical origin. `options.selection` seeds the selection
    /// engine - from then on the live engine rect drives the dim
    /// cutout.
    ///
    /// # Errors
    ///
    /// Same session/event-loop failures as [`Self::new`].
    pub fn with_capture(capture: FrozenCapture, options: BackdropOptions) -> Result<Self, UiError> {
        Self::with_capture_configured(capture, options, SelectionConfig::default())
    }

    /// [`Self::with_capture`] with explicit selection-engine config (the
    /// `[editor]` keys: HUD position/hide-time, double-click copy).
    ///
    /// # Errors
    ///
    /// Same session/event-loop failures as [`Self::new`].
    pub fn with_capture_configured(
        capture: FrozenCapture,
        options: BackdropOptions,
        config: SelectionConfig,
    ) -> Result<Self, UiError> {
        require_display_server()?;
        let event_loop = EventLoop::<UiEvent>::with_user_event().build()?;
        let handle = OverlayHandle {
            proxy: event_loop.create_proxy(),
        };
        let mut app = OverlayApp::new();
        app.backdrop = Some(Backdrop::plan(capture, &DesignTokens::default()));
        app.backdrop_options = options;
        app.core.selection_mut().configure(config);
        app.core.selection_mut().set_rect(options.selection);
        Ok(Self {
            event_loop,
            handle,
            app,
        })
    }

    /// The overlay's headless core before [`Self::run`] (the preselect seam:
    /// launch flows seed the selection, cascade flags, or config).
    pub fn core_mut(&mut self) -> &mut OverlayCore {
        &mut self.app.core
    }

    /// Re-themes the planned backdrop (dim layer + letterbox color) from
    /// the core's live chrome tokens - call AFTER `configure_core` has
    /// projected the `[ui]` config, since [`Backdrop::plan`](crate::Backdrop::plan)
    /// ran on the pre-config default tokens at build time.
    pub fn retheme_backdrop(&mut self) {
        let tokens = self.app.core.chrome().tokens().clone();
        if let Some(backdrop) = self.app.backdrop.as_mut() {
            backdrop.retheme(&tokens);
        }
    }

    /// Registers the binary-layer window-attributes hook (the session's
    /// shell-facing name - Wayland `app_id` / X11 `WM_CLASS` - applied by
    /// the daemon's session child, a recorded shell deviation; the
    /// `pins::WindowCustomizer` precedent keeps this crate platform-pure).
    #[must_use]
    pub fn with_window_customizer(mut self, customizer: crate::pins::WindowCustomizer) -> Self {
        self.app.customizer = Some(customizer);
        self
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

/// Verifies a display-server session is present before touching winit.
///
/// Portable environment probe (purity gate: no platform imports in this
/// crate): `WAYLAND_DISPLAY` is the standard session variable every Wayland
/// compositor exports to its clients, `DISPLAY` the X11 equivalent - the
/// interactive UI runs on both session types (X11 Phase B). With NEITHER
/// variable set there is no session to open a window on, so the gate is a
/// typed startup error with a hint - never a silent platform switch.
pub(crate) fn require_display_server() -> Result<(), UiError> {
    if has_display_server(
        std::env::var_os("WAYLAND_DISPLAY").as_deref(),
        std::env::var_os("DISPLAY").as_deref(),
    ) {
        Ok(())
    } else {
        Err(UiError::NoDisplayServer)
    }
}

/// The gate rule with the environment values injectable (headless tests):
/// either session variable set and non-empty means a display server is
/// reachable. Empty values count as unset (the `flowshot-actions`
/// `session_from_env` rule this gate mirrors).
fn has_display_server(
    wayland_display: Option<&std::ffi::OsStr>,
    display: Option<&std::ffi::OsStr>,
) -> bool {
    let set = |value: Option<&std::ffi::OsStr>| value.is_some_and(|value| !value.is_empty());
    set(wayland_display) || set(display)
}

#[cfg(test)]
mod tests {
    use super::has_display_server;
    use std::ffi::OsStr;

    #[test]
    fn gate_passes_on_a_wayland_session_variable() {
        assert!(has_display_server(Some(OsStr::new("wayland-0")), None));
    }

    #[test]
    fn gate_passes_on_an_x11_display_variable_alone() {
        assert!(has_display_server(None, Some(OsStr::new(":0"))));
    }

    #[test]
    fn gate_passes_on_an_xwayland_session_exporting_both() {
        assert!(has_display_server(
            Some(OsStr::new("wayland-0")),
            Some(OsStr::new(":1"))
        ));
    }

    #[test]
    fn gate_passes_when_only_the_empty_variable_is_dropped() {
        // An empty WAYLAND_DISPLAY counts as unset, but DISPLAY still
        // carries the session (the empty=unset rule).
        assert!(has_display_server(
            Some(OsStr::new("")),
            Some(OsStr::new(":0"))
        ));
    }

    #[test]
    fn gate_rejects_when_neither_variable_is_set() {
        assert!(!has_display_server(None, None));
    }

    #[test]
    fn gate_rejects_when_both_variables_are_empty() {
        assert!(!has_display_server(
            Some(OsStr::new("")),
            Some(OsStr::new(""))
        ));
    }
}
