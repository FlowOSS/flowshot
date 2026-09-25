//! Typed errors for connecting to, probing, and capturing on a Wayland
//! session.

use std::time::Duration;

use flowshot_capture::{BackendKind, CaptureError};
use flowshot_core::geometry::GeometryError;
use thiserror::Error;

/// Why establishing the Wayland capture session failed.
#[derive(Debug, Error)]
pub enum ConnectError {
    /// The Wayland compositor socket could not be connected. The `hint`
    /// explains the most likely cause based on the process environment.
    #[error("could not connect to the Wayland compositor: {source} (hint: {hint})")]
    Socket {
        /// The underlying connection error from `wayland-client`.
        source: wayland_client::ConnectError,
        /// Human-readable remediation hint derived from the environment.
        hint: String,
    },
    /// The socket connected, but session setup (registry probe, output
    /// collection, event loop construction) failed.
    #[error("Wayland session setup failed: {0}")]
    Setup(#[from] ProbeError),
    /// The capture thread did not report session setup completion in time.
    #[error("the Wayland capture thread did not finish session setup within {timeout:?}")]
    StartupTimeout {
        /// The startup deadline that expired.
        timeout: Duration,
    },
    /// The operating system refused to spawn the capture thread.
    #[error("the Wayland capture thread could not be spawned: {0}")]
    ThreadSpawn(#[from] std::io::Error),
}

/// Why an in-session probe or snapshot request failed.
#[derive(Debug, Error)]
pub enum ProbeError {
    /// The compositor sent a protocol error or the connection broke while
    /// dispatching events.
    #[error("Wayland protocol error during session probe: {0}")]
    Dispatch(#[from] wayland_client::DispatchError),
    /// The `calloop` event loop infrastructure could not be set up or ran
    /// into a polling error.
    #[error("the capture event loop failed: {0}")]
    EventLoop(#[from] calloop::Error),
    /// The capture thread did not answer a request in time.
    #[error("the Wayland capture thread did not answer within {timeout:?}")]
    Timeout {
        /// The reply deadline that expired.
        timeout: Duration,
    },
    /// The capture thread exited (or its channel closed) before answering.
    #[error("the Wayland capture thread closed the session before answering")]
    ThreadClosed,
}

/// Why the compositor reported an `ext-image-copy-capture-v1` frame as
/// failed; mirrors the protocol's `failure_reason` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameFailure {
    /// An unspecified runtime error; the protocol notes the client may retry.
    Unknown,
    /// The attached buffer did not match the session's latest constraints;
    /// the client should re-allocate and retry.
    BufferConstraints,
    /// The capture session is no longer available.
    Stopped,
}

/// Why an `ext-image-copy-capture-v1` capture failed.
///
/// Every failure of the one-shot capture chain (connect, collect, source,
/// session constraints, buffer allocation, frame copy, readback) is a
/// variant here; [`CaptureError`] conversion maps [`IccError::Timeout`] onto
/// [`CaptureError::Timeout`] and everything else onto
/// [`CaptureError::Backend`] with this error as the source.
#[derive(Debug, Error)]
pub enum IccError {
    /// The one-shot capture connection to the compositor could not be
    /// established.
    #[error("ext-image-copy-capture could not connect: {0}")]
    Connect(#[from] ConnectError),
    /// Registry and output collection on the capture connection failed.
    #[error("ext-image-copy-capture session collection failed: {0}")]
    Collection(#[from] ProbeError),
    /// A protocol global required for capture was not advertised.
    #[error("the compositor does not advertise {0}")]
    MissingProtocol(&'static str),
    /// The session sees no outputs, so there is nothing to capture.
    #[error("the Wayland session has no outputs to capture")]
    NoOutputs,
    /// No output matches the requested connector name.
    #[error("no output named {requested}; available connectors: {available:?}")]
    OutputNotFound {
        /// The connector name that was requested.
        requested: String,
        /// The connector names the session actually advertises.
        available: Vec<String>,
    },
    /// The compositor stopped the capture session before a frame arrived.
    #[error("the compositor stopped the capture session")]
    SessionStopped,
    /// The compositor reported the capture frame as failed.
    #[error("the compositor failed the capture frame: {reason:?}")]
    FrameFailed {
        /// The protocol's failure reason.
        reason: FrameFailure,
    },
    /// The frame failed with a reason value this protocol version does not
    /// define.
    #[error("the compositor failed the capture frame with unknown reason {0}")]
    UnknownFailureReason(u32),
    /// The session advertised no shared-memory format this backend supports
    /// in v1 (`XRGB8888`, `ARGB8888`, or `RGBA8888`).
    #[error("no supported shm format among the advertised {advertised:?}")]
    NoSupportedFormat {
        /// The raw `wl_shm` format values the session advertised.
        advertised: Vec<u32>,
    },
    /// The constraint batch was incomplete: no `done`, no buffer size, or a
    /// zero dimension.
    #[error("the capture session reported incomplete buffer constraints")]
    IncompleteConstraints,
    /// The captured buffer size does not match the output's post-transform
    /// physical size from enumeration (stale metadata or a mode change
    /// mid-capture).
    #[error("captured buffer {reported:?} does not match output buffer size {expected:?}")]
    BufferSizeMismatch {
        /// The size the capture session reported.
        reported: (u32, u32),
        /// The post-transform size enumeration predicted.
        expected: (i32, i32),
    },
    /// The compositor did not complete a capture phase within its deadline.
    /// On `Hyprland` this also covers a pending permission popup: the frame
    /// is withheld until the user decides, and this backend never waits
    /// longer than the deadline.
    #[error("ext-image-copy-capture did not complete within {timeout:?}")]
    Timeout {
        /// The per-phase deadline that expired.
        timeout: Duration,
    },
    /// An operating-system call failed (anonymous file creation, sizing,
    /// polling, or pixel readback).
    #[error("an OS call failed during capture: {0}")]
    Io(#[from] std::io::Error),
    /// The Wayland connection broke while capture requests or reads were in
    /// flight.
    #[error("the Wayland transport failed during capture: {0}")]
    Transport(#[from] wayland_client::backend::WaylandError),
    /// The compositor sent a protocol error or an unparseable message during
    /// capture.
    #[error("Wayland protocol error during capture: {0}")]
    Protocol(#[from] wayland_client::DispatchError),
    /// Pixel geometry validation failed while normalizing a captured frame.
    #[error("capture frame geometry is invalid: {0}")]
    Geometry(#[from] GeometryError),
    /// The compositor delivered a permission-denial black frame (the
    /// `Hyprland` enforce-permissions denial image); capture must not
    /// proceed and the caller surfaces a notification.
    #[error("screen capture permission was denied by the compositor")]
    PermissionDenied,
    /// An internal invariant was violated (arithmetic overflow, a settled
    /// state that was not settled).
    #[error("internal ext-image-copy-capture error: {0}")]
    Internal(&'static str),
}

impl From<IccError> for CaptureError {
    fn from(error: IccError) -> Self {
        match error {
            IccError::Timeout { .. } => CaptureError::Timeout {
                backend: BackendKind::ExtImageCopyCapture,
            },
            other => CaptureError::Backend {
                backend: BackendKind::ExtImageCopyCapture,
                source: Box::new(other),
            },
        }
    }
}

/// Why a `wlr-screencopy-unstable-v1` capture failed.
///
/// Every failure of the one-shot screencopy chain (connect, collect, frame
/// creation, buffer constraints, `wl_shm` allocation, copy, readback) is a
/// variant here; the [`CaptureError`] conversion maps
/// [`ScreencopyError::Timeout`] onto [`CaptureError::Timeout`] and everything
/// else onto [`CaptureError::Backend`] tagged [`BackendKind::WlrScreencopy`].
///
/// Unlike [`IccError`], the screencopy `failed` event carries no reason value
/// and the protocol has no separate session/stopped lifecycle, so those
/// variants are absent.
#[derive(Debug, Error)]
pub enum ScreencopyError {
    /// The one-shot capture connection to the compositor could not be
    /// established.
    #[error("wlr-screencopy could not connect: {0}")]
    Connect(#[from] ConnectError),
    /// Registry and output collection on the capture connection failed.
    #[error("wlr-screencopy session collection failed: {0}")]
    Collection(#[from] ProbeError),
    /// A protocol global required for capture was not advertised.
    #[error("the compositor does not advertise {0}")]
    MissingProtocol(&'static str),
    /// The session sees no outputs, so there is nothing to capture.
    #[error("the Wayland session has no outputs to capture")]
    NoOutputs,
    /// No output matches the requested connector name.
    #[error("no output named {requested}; available connectors: {available:?}")]
    OutputNotFound {
        /// The connector name that was requested.
        requested: String,
        /// The connector names the session actually advertises.
        available: Vec<String>,
    },
    /// The compositor reported the screencopy frame as failed. The protocol
    /// carries no reason value, so a retry is the only client recourse.
    #[error("the compositor failed the screencopy frame")]
    FrameFailed,
    /// The frame's single `buffer` event advertised a shared-memory format
    /// this backend does not support in v1 (`XRGB8888`, `ARGB8888`, or
    /// `RGBA8888`).
    #[error("the compositor advertised unsupported shm format {advertised:#010x}")]
    NoSupportedFormat {
        /// The raw `wl_shm` format wire value the frame advertised.
        advertised: u32,
    },
    /// The buffer constraints were incomplete: no `buffer_done`, no `buffer`
    /// event, or a zero dimension.
    #[error("the screencopy frame reported incomplete buffer constraints")]
    IncompleteConstraints,
    /// The captured buffer size does not match the output's native
    /// (pre-transform) physical size from enumeration. `wlr-screencopy`
    /// delivers buffers in the output's native orientation, so the guard
    /// compares against [`OutputInfo::physical_size`], not the post-transform
    /// [`OutputInfo::buffer_size`].
    ///
    /// [`OutputInfo::physical_size`]: flowshot_core::geometry::OutputInfo::physical_size
    /// [`OutputInfo::buffer_size`]: flowshot_core::geometry::OutputInfo::buffer_size
    #[error("captured buffer {reported:?} does not match output physical size {expected:?}")]
    BufferSizeMismatch {
        /// The size the screencopy frame reported.
        reported: (u32, u32),
        /// The native physical size enumeration predicted.
        expected: (i32, i32),
    },
    /// The compositor did not complete a capture phase within its deadline.
    #[error("wlr-screencopy did not complete within {timeout:?}")]
    Timeout {
        /// The per-phase deadline that expired.
        timeout: Duration,
    },
    /// An operating-system call failed (anonymous file creation, sizing,
    /// polling, or pixel readback).
    #[error("an OS call failed during screencopy capture: {0}")]
    Io(#[from] std::io::Error),
    /// The Wayland connection broke while capture requests or reads were in
    /// flight.
    #[error("the Wayland transport failed during screencopy capture: {0}")]
    Transport(#[from] wayland_client::backend::WaylandError),
    /// The compositor sent a protocol error or an unparseable message during
    /// capture.
    #[error("Wayland protocol error during screencopy capture: {0}")]
    Protocol(#[from] wayland_client::DispatchError),
    /// Pixel geometry validation failed while normalizing a captured frame.
    #[error("screencopy frame geometry is invalid: {0}")]
    Geometry(#[from] GeometryError),
    /// The compositor delivered a permission-denial black frame (the
    /// `Hyprland` enforce-permissions screencopy denial image); capture must
    /// not proceed and the caller surfaces a notification.
    #[error("screen capture permission was denied by the compositor")]
    PermissionDenied,
    /// An internal invariant was violated (arithmetic overflow, a settled
    /// state that was not settled).
    #[error("internal wlr-screencopy error: {0}")]
    Internal(&'static str),
}

impl From<ScreencopyError> for CaptureError {
    fn from(error: ScreencopyError) -> Self {
        match error {
            ScreencopyError::Timeout { .. } => CaptureError::Timeout {
                backend: BackendKind::WlrScreencopy,
            },
            other => CaptureError::Backend {
                backend: BackendKind::WlrScreencopy,
                source: Box::new(other),
            },
        }
    }
}

/// The shared infrastructure contract for one-shot capture backend errors.
///
/// The deadline-bounded dispatch machinery ([`crate::icc::wait`]) and the
/// worker-thread bridge ([`crate::worker`]) produce transport, timeout, and
/// internal failures that are independent of any specific compositor protocol.
/// This trait lets those helpers stay generic over the concrete backend error
/// ([`IccError`], [`ScreencopyError`]) so each failure is tagged with the
/// correct [`BackendKind`] when lifted into a [`CaptureError`].
pub(crate) trait BackendError:
    std::error::Error
    + Send
    + Sync
    + 'static
    + From<std::io::Error>
    + From<wayland_client::backend::WaylandError>
    + From<wayland_client::DispatchError>
{
    /// The backend this error type reports failures for.
    const KIND: BackendKind;
    /// Builds the timeout error for an expired per-phase deadline.
    fn timeout(timeout: Duration) -> Self;
    /// Builds an internal-invariant error.
    fn internal(message: &'static str) -> Self;
}

impl BackendError for IccError {
    const KIND: BackendKind = BackendKind::ExtImageCopyCapture;

    fn timeout(timeout: Duration) -> Self {
        Self::Timeout { timeout }
    }

    fn internal(message: &'static str) -> Self {
        Self::Internal(message)
    }
}

impl BackendError for ScreencopyError {
    const KIND: BackendKind = BackendKind::WlrScreencopy;

    fn timeout(timeout: Duration) -> Self {
        Self::Timeout { timeout }
    }

    fn internal(message: &'static str) -> Self {
        Self::Internal(message)
    }
}

/// Wraps a `wayland-client` connect failure with an environment-based hint.
pub(crate) fn socket_connect_error(source: wayland_client::ConnectError) -> ConnectError {
    let hint = connect_hint(
        std::env::var_os("WAYLAND_DISPLAY").as_deref(),
        std::env::var_os("XDG_RUNTIME_DIR").as_deref(),
    );
    ConnectError::Socket { source, hint }
}

/// Builds the remediation hint for a failed socket connection.
///
/// Pure function of the two environment values `wayland-client` consults, so
/// the message logic is testable without touching the process environment.
fn connect_hint(
    wayland_display: Option<&std::ffi::OsStr>,
    runtime_dir: Option<&std::ffi::OsStr>,
) -> String {
    match (wayland_display, runtime_dir) {
        (_, None) => "XDG_RUNTIME_DIR is not set; a Wayland session must export it \
                      (this process is probably not running inside the session)"
            .to_owned(),
        (None, _) => "WAYLAND_DISPLAY is not set; are you running inside a Wayland session? \
                      (under X11 or a TTY, Wayland capture is unavailable)"
            .to_owned(),
        (Some(display), Some(dir)) => format!(
            "WAYLAND_DISPLAY={} does not name a live compositor socket under XDG_RUNTIME_DIR={}; \
             is the compositor running?",
            display.to_string_lossy(),
            dir.to_string_lossy()
        ),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn hint_names_missing_runtime_dir_first() {
        let hint = connect_hint(Some("wayland-1".as_ref()), None);
        assert!(hint.contains("XDG_RUNTIME_DIR is not set"), "{hint}");
    }

    #[test]
    fn hint_names_missing_wayland_display() {
        let hint = connect_hint(None, Some("/run/user/1000".as_ref()));
        assert!(hint.contains("WAYLAND_DISPLAY is not set"), "{hint}");
    }

    #[test]
    fn hint_echoes_both_values_when_set() {
        let hint = connect_hint(
            Some("nonexistent".as_ref()),
            Some("/run/user/1000".as_ref()),
        );
        assert!(hint.contains("WAYLAND_DISPLAY=nonexistent"), "{hint}");
        assert!(hint.contains("/run/user/1000"), "{hint}");
    }

    #[test]
    fn socket_error_display_includes_hint() {
        let err = ConnectError::Socket {
            source: wayland_client::ConnectError::NoCompositor,
            hint: "check the session".to_owned(),
        };
        let text = err.to_string();
        assert!(text.contains("check the session"), "{text}");
        assert!(text.contains("Could not find wayland compositor"), "{text}");
    }
}
