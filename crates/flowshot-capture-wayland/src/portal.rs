//! The `org.freedesktop.portal` capture backends: [`PortalScreenshotBackend`]
//! ([`BackendKind::PortalScreenshot`], ladder rung 5) and
//! [`PortalScreenCastBackend`] ([`BackendKind::PortalScreenCast`], rung 4).
//!
//! The portals are the universal fallback: they work on every desktop with a
//! portal frontend (`XDPH` on `Hyprland`, `mutter` on GNOME, the GTK
//! frontend elsewhere) without compositor-protocol support. Both backends
//! follow the crate's one-shot worker discipline ([`spawn_worker`]): every
//! operation runs on a short-lived worker thread, the async `D-Bus` half on
//! a thread-private `tokio` runtime, the `PipeWire` half on its own blocking
//! main loop - so the shared [`CaptureBackend`] contract stays drivable from
//! any executor and nothing parks between captures.
//!
//! Capability gating lives in [`probe`]: portal availability is a session-bus
//! check the Wayland registry cannot see, and [`PortalAvailability::observe_into`]
//! extends the shared [`CapabilityProbe`] so the negotiation ladder orders
//! the portal rungs exactly as the native ones.
//!
//! # Verification class
//!
//! `Hyprland`/`XDPH` portal paths are live-verified on the QA session;
//! GNOME-specific behaviors (the `mutter` picker UX, shell-UI delegation,
//! upright stream orientation) are compile- and unit-tested only and are
//! NEVER claimed live-verified.
//!
//! [`spawn_worker`]: crate::worker::spawn_worker
//! [`CaptureBackend`]: flowshot_capture::CaptureBackend
//! [`probe`]: crate::portal::probe

pub(crate) mod composite;
pub(crate) mod error;
pub(crate) mod pipewire;
pub(crate) mod probe;
pub(crate) mod run;
pub(crate) mod screencast;
pub(crate) mod screenshot;
pub(crate) mod streams;

use flowshot_capture::{BackendKind, CaptureError, CaptureOpts, Frame};

pub use error::{PortalDenial, PortalErrorKind, PortalScreenCastError, PortalScreenshotError};
pub use probe::{PortalAvailability, probe_portals, probe_portals_blocking};
use run::Selection;

/// The `org.freedesktop.portal.Screenshot` capture backend (ladder rung 5).
///
/// Non-interactive by default: the portal frontend captures the full layout
/// (`XDPH` shells out to `grim`) and the backend crops per-output frames
/// through the detected pixel space. The [`new_interactive`] variant is the
/// GNOME rung-5 fallback UX: the compositor's picker selects a region whose
/// image goes straight to the editor (overlay skipped).
///
/// Stateless and cheap to construct; every operation opens its own one-shot
/// connections on a worker thread.
///
/// [`new_interactive`]: PortalScreenshotBackend::new_interactive
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PortalScreenshotBackend {
    pub(crate) interactive: bool,
}

impl PortalScreenshotBackend {
    /// Creates the non-interactive backend (full-layout composite).
    #[must_use]
    pub const fn new() -> Self {
        Self { interactive: false }
    }

    /// Creates the interactive backend (compositor picker -> region image,
    /// the ladder rung-5 GNOME flow).
    #[must_use]
    pub const fn new_interactive() -> Self {
        Self { interactive: true }
    }

    /// Whether requests go out with `interactive: true`.
    #[must_use]
    pub const fn is_interactive(&self) -> bool {
        self.interactive
    }

    /// Captures the single output with this connector name (QA harnesses and
    /// single-output callers).
    ///
    /// # Errors
    ///
    /// [`CaptureError::Backend`] wrapping
    /// [`PortalScreenshotError`] (`OutputNotFound` listing the available
    /// connectors) when the session does not advertise `connector`, plus
    /// every error of [`CaptureBackend::capture_outputs`](flowshot_capture::CaptureBackend::capture_outputs).
    pub async fn capture_output_named(&self, connector: &str) -> Result<Frame, CaptureError> {
        let interactive = self.interactive;
        let name = connector.to_owned();
        let captured = crate::worker::spawn_worker("flowshot-portal-screenshot-named", move || {
            screenshot::screenshot_run(interactive, &Selection::Named(name))
        })
        .await?;
        first_frame(captured, BackendKind::PortalScreenshot)
    }
}

/// The `org.freedesktop.portal.ScreenCast` single-frame backend (ladder
/// rung 4).
///
/// Captures through a portal screencast session and `PipeWire`: the first
/// frame of every stream is the capture, then the session and every
/// `PipeWire` object are torn down (no restore token - one-shot by design).
/// Implementations that share one source per session (`XDPH`) get one
/// session per missing output, bounded by the output count.
///
/// Stateless and cheap to construct; every operation opens its own one-shot
/// connections on a worker thread.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PortalScreenCastBackend;

impl PortalScreenCastBackend {
    /// Creates the backend.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Captures the single output with this connector name (QA harnesses and
    /// single-output callers). The portal's picker still decides the shared
    /// source; the frame is matched to `connector` and a mismatch is a typed
    /// error.
    ///
    /// # Errors
    ///
    /// [`CaptureError::Backend`] wrapping [`PortalScreenCastError`]
    /// (`OutputNotFound`, `StreamUnmapped`, `OutputsUncaptured`) plus every
    /// error of [`CaptureBackend::capture_outputs`](flowshot_capture::CaptureBackend::capture_outputs).
    pub async fn capture_output_named(
        &self,
        connector: &str,
        opts: CaptureOpts,
    ) -> Result<Frame, CaptureError> {
        let name = connector.to_owned();
        let captured = crate::worker::spawn_worker("flowshot-portal-screencast-named", move || {
            screencast::screencast_run(opts, &Selection::Named(name))
        })
        .await?;
        first_frame(
            screenshot::PortalCapture::Outputs(captured),
            BackendKind::PortalScreenCast,
        )
    }
}

/// Extracts the single frame of a named capture.
fn first_frame(
    captured: screenshot::PortalCapture,
    kind: BackendKind,
) -> Result<Frame, CaptureError> {
    let mut frames = captured.into_frames();
    if frames.len() == 1 {
        return Ok(frames.remove(0));
    }
    Err(CaptureError::Backend {
        backend: kind,
        source: PortalErrorKind::Internal("a named capture did not return exactly one frame")
            .into(),
    })
}
