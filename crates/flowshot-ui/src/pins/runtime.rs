//! Pin runtime entry point: event-loop ownership and cross-thread control
//! (the [`crate::OverlayRuntime`] shape, for floating pin windows).
//!
//! Pins OUTLIVE the capture overlay: the overlay tears down after the
//! capture session while pins stay up in the hosting process (CLI one-shot
//! or daemon), so they own a separate winit event loop. One
//! loop hosts every pin window (multi-pin); the loop exits when the last
//! pin closes.

use winit::event_loop::{EventLoop, EventLoopProxy};
use winit::window::WindowAttributes;

#[cfg(feature = "test-drive")]
use super::event::PinInput;
use super::image::PinImage;
use super::shell::PinApp;
use super::sink::{PinActionSink, PinId};
use super::state::PinBehavior;
use crate::error::UiError;
use crate::runtime::require_display_server;

/// One pin to spawn: a host-assigned id and its upright RGBA image.
#[derive(Debug, Clone)]
pub struct PinSpec {
    /// Host-assigned identity (bridges to the actions-crate registry by
    /// raw value).
    pub id: PinId,
    /// The pinned image (upright orientation, physical pixels).
    pub image: PinImage,
}

/// Binary-layer hook applied to every pin window's attributes before
/// creation. The lib crate stays platform-pure; the binary layer or a
/// QA harness uses this to set the Wayland `app_id` (`flowshot-pin`) via
/// `winit::platform::wayland::WindowAttributesExtWayland` - the same seam
/// the overlay shell defers `app_id` to.
#[derive(Clone)]
pub struct WindowCustomizer(
    std::sync::Arc<dyn Fn(WindowAttributes) -> WindowAttributes + Send + Sync + 'static>,
);

impl WindowCustomizer {
    /// Wraps an attribute-customization function.
    pub fn new(
        customize: impl Fn(WindowAttributes) -> WindowAttributes + Send + Sync + 'static,
    ) -> Self {
        Self(std::sync::Arc::new(customize))
    }

    /// Applies the hook to `attributes`.
    #[must_use]
    pub fn apply(&self, attributes: WindowAttributes) -> WindowAttributes {
        (self.0)(attributes)
    }
}

impl std::fmt::Debug for WindowCustomizer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WindowCustomizer(..)")
    }
}

/// Events sent into the running pin loop from other threads.
#[derive(Debug, Clone)]
pub(crate) enum PinUiEvent {
    /// Gracefully close every pin window.
    Exit,
    /// Synthetic input injected through the `test-drive` seam.
    #[cfg(feature = "test-drive")]
    Synthetic {
        /// The target pin.
        id: PinId,
        /// The event to route.
        input: PinInput,
    },
}

/// Thread-safe handle to a running pin host.
#[derive(Debug, Clone)]
pub struct PinHandle {
    proxy: EventLoopProxy<PinUiEvent>,
}

impl PinHandle {
    /// Requests graceful teardown of every pin window.
    ///
    /// # Errors
    ///
    /// Returns [`UiError::EventLoopClosed`] when the loop already exited.
    pub fn request_exit(&self) -> Result<(), UiError> {
        self.proxy
            .send_event(PinUiEvent::Exit)
            .map_err(|_| UiError::EventLoopClosed)
    }

    /// TEST SEAM (feature `test-drive`): injects a synthetic input event
    /// into the pin's exact production routing path - wheel zoom, keys,
    /// buttons, and touch become QA-able without external injection tools
    /// (wtype/ydotool are absent on the QA machine, issues.md 2026-09-25).
    ///
    /// # Errors
    ///
    /// Returns [`UiError::EventLoopClosed`] when the loop already exited.
    #[cfg(feature = "test-drive")]
    pub fn inject_event(&self, id: PinId, input: PinInput) -> Result<(), UiError> {
        self.proxy
            .send_event(PinUiEvent::Synthetic { id, input })
            .map_err(|_| UiError::EventLoopClosed)
    }
}

/// The multi-pin window host.
///
/// # Example (startup shape)
///
/// ```no_run
/// use flowshot_ui::pins::{PinBehavior, PinImage, PinRuntime, PinSpec, PinId};
///
/// fn main() -> Result<(), flowshot_ui::UiError> {
///     let image = PinImage::new(2, 1, vec![255; 8])?;
///     let runtime = PinRuntime::new(
///         vec![PinSpec { id: PinId::new(1), image }],
///         None,
///         PinBehavior::default(),
///     )?;
///     runtime.run()
/// }
/// ```
#[derive(Debug)]
pub struct PinRuntime {
    event_loop: EventLoop<PinUiEvent>,
    handle: PinHandle,
    app: PinApp,
}

impl PinRuntime {
    /// Creates the pin host for `specs`. Windows and GPU objects are
    /// created on `Resumed` inside [`Self::run`].
    ///
    /// # Errors
    ///
    /// Returns [`UiError::NoDisplayServer`] when no compositor session is
    /// detected, [`UiError::EventLoop`] when the event loop cannot be
    /// created, and [`UiError::NoPinsRequested`] when `specs` is empty (a
    /// pin host without pins would spin forever).
    pub fn new(
        specs: Vec<PinSpec>,
        sink: Option<std::sync::Arc<dyn PinActionSink>>,
        behavior: PinBehavior,
    ) -> Result<Self, UiError> {
        if specs.is_empty() {
            return Err(UiError::NoPinsRequested);
        }
        require_display_server()?;
        let event_loop = EventLoop::<PinUiEvent>::with_user_event().build()?;
        let handle = PinHandle {
            proxy: event_loop.create_proxy(),
        };
        Ok(Self {
            event_loop,
            handle,
            app: PinApp::new(specs, sink, behavior),
        })
    }

    /// Registers the binary-layer window-attribute hook (Wayland `app_id`).
    #[must_use]
    pub fn with_window_customizer(mut self, customizer: WindowCustomizer) -> Self {
        self.app.customizer = Some(customizer);
        self
    }

    /// The cross-thread control handle (exit; `test-drive` injection).
    #[must_use]
    pub const fn handle(&self) -> &PinHandle {
        &self.handle
    }

    /// Runs the event loop until every pin closed; consumes the runtime.
    ///
    /// # Errors
    ///
    /// Returns the typed fatal error when startup failed inside the loop
    /// (window/GPU/surface creation) and [`UiError::EventLoop`] when the
    /// loop itself errored. A clean all-pins-closed teardown returns `Ok`.
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
