//! Typed errors for the `org.kde.KWin.ScreenShot2` capture backend.
//!
//! [`KwinError`] is the backend's failure vocabulary and implements the
//! shared [`BackendError`] contract, so the worker bridge and deadline
//! machinery tag every failure with [`BackendKind::KwinScreenShot2`].
//! [`DecodeError`] isolates the wire-contract decode failures (metadata
//! vardict parsing, `QImage` format mapping, raw payload integrity) so
//! callers can distinguish "`KWin` said something this client cannot parse"
//! from transport failures.

use std::time::Duration;

use flowshot_capture::{BackendKind, CaptureError};
use flowshot_core::geometry::GeometryError;
use thiserror::Error;

use crate::error::BackendError;

/// Why decoding a `ScreenShot2` reply or its raw pipe payload failed.
///
/// The wire contract is pinned to the fetched `KWin` source (cited in the
/// `kwin` module docs): the reply vardict carries `{type:"raw", format,
/// width, height, stride, scale}` plus additive keys the parser ignores,
/// and the pipe carries exactly `stride * height` raw `QImage` bytes.
#[derive(Debug, Error)]
pub enum DecodeError {
    /// The reply's `type` key announced a payload encoding this client does
    /// not speak (the contract pins `raw`).
    #[error("the ScreenShot2 reply announced unsupported payload type {found:?}")]
    UnsupportedType {
        /// The `type` value the reply carried.
        found: String,
    },
    /// The reply's `format` value is not a `QImage::Format` this backend
    /// maps onto a [`FrameFormat`](flowshot_capture::FrameFormat).
    #[error("the ScreenShot2 reply announced unsupported QImage format {format}")]
    UnknownFormat {
        /// The raw `QImage::Format` enum value.
        format: u32,
    },
    /// A required metadata key was absent from the reply vardict.
    #[error("the ScreenShot2 reply metadata is missing the {key:?} key")]
    MissingKey {
        /// The required key that was absent.
        key: &'static str,
    },
    /// A metadata key carried the wrong value type.
    #[error("the ScreenShot2 reply metadata key {key:?} has the wrong value type")]
    InvalidKeyType {
        /// The key whose value type did not match the contract.
        key: &'static str,
    },
    /// The reply body did not deserialize as the contract's `a{sv}` vardict.
    #[error("the ScreenShot2 reply body is not a metadata vardict: {source}")]
    MalformedReply {
        /// The underlying `zbus` deserialization error.
        #[source]
        source: zbus::Error,
    },
    /// The pipe reached end-of-file before the metadata's `stride * height`
    /// bytes arrived (`KWin`'s writer aborted mid-payload).
    #[error("the ScreenShot2 pipe delivered {received} of {expected} payload bytes")]
    TruncatedPayload {
        /// The byte count the metadata announced.
        expected: usize,
        /// The byte count actually received before end-of-file.
        received: usize,
    },
    /// The metadata's geometry is internally inconsistent (a zero dimension
    /// or a stride narrower than `width * 4`).
    #[error("the ScreenShot2 reply geometry is invalid: {width}x{height} stride {stride}")]
    InvalidGeometry {
        /// The announced width in pixels.
        width: u32,
        /// The announced height in pixels.
        height: u32,
        /// The announced bytes per row.
        stride: u32,
    },
    /// The metadata's `stride * height` exceeds the sanity bound for a real
    /// capture, so the values are corrupt and the buffer is never allocated.
    #[error("the ScreenShot2 reply announces an implausible {bytes}-byte payload")]
    PayloadTooLarge {
        /// The announced payload size in bytes.
        bytes: u64,
    },
}

/// Why a `org.kde.KWin.ScreenShot2` capture failed.
///
/// [`CaptureError`] conversion maps [`KwinError::Timeout`] onto
/// [`CaptureError::Timeout`] and everything else onto
/// [`CaptureError::Backend`] tagged [`BackendKind::KwinScreenShot2`].
#[derive(Debug, Error)]
pub enum KwinError {
    /// The one-shot Wayland connection used for output geometry could not be
    /// established.
    #[error("KWin capture could not connect to the Wayland session: {0}")]
    Connect(#[from] crate::error::ConnectError),
    /// Registry and output collection on the Wayland connection failed.
    #[error("KWin capture session collection failed: {0}")]
    Collection(#[from] crate::error::ProbeError),
    /// The Wayland session sees no outputs, so there is nothing to capture.
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
    /// `KWin` cancelled the capture (the interactive picker was dismissed):
    /// `org.kde.KWin.ScreenShot2.Error.Cancelled`.
    #[error("the KWin screenshot was cancelled")]
    Cancelled,
    /// `KWin` rejected the request with one of its named
    /// `org.kde.KWin.ScreenShot2.Error.*` errors.
    #[error("KWin rejected the screenshot request: {name}")]
    KwinReply {
        /// The `D-Bus` error name `KWin` sent.
        name: String,
        /// The human-readable detail `KWin` attached, when present.
        message: Option<String>,
    },
    /// A `D-Bus` call failed (transport, name resolution, or a method error
    /// outside the `ScreenShot2` error family).
    #[error("the ScreenShot2 D-Bus call failed: {source}")]
    Dbus {
        /// The underlying `zbus` error.
        #[source]
        source: zbus::Error,
    },
    /// The reply metadata or the raw pipe payload violated the wire
    /// contract.
    #[error("the ScreenShot2 reply could not be decoded: {0}")]
    Decode(#[from] DecodeError),
    /// The captured area's dimensions do not match the output's
    /// post-transform physical size (stale geometry, a rejected
    /// `native-resolution` flag, or a mid-capture mode change).
    #[error("captured area {reported:?} does not match output buffer size {expected:?}")]
    BufferSizeMismatch {
        /// The size the reply metadata reported.
        reported: (u32, u32),
        /// The post-transform size enumeration predicted.
        expected: (i32, i32),
    },
    /// A phase did not complete within its deadline. For interactive
    /// captures this also covers a picker the user never answered.
    #[error("the KWin ScreenShot2 service did not complete within {timeout:?}")]
    Timeout {
        /// The phase budget that expired.
        timeout: Duration,
    },
    /// An operating-system call failed (pipe creation, polling, or reading).
    #[error("an OS call failed during KWin capture: {0}")]
    Io(#[from] std::io::Error),
    /// The Wayland connection broke while output geometry was collected.
    #[error("the Wayland transport failed during KWin capture: {0}")]
    Transport(#[from] wayland_client::backend::WaylandError),
    /// The compositor sent a protocol error while output geometry was
    /// collected.
    #[error("Wayland protocol error during KWin capture: {0}")]
    Protocol(#[from] wayland_client::DispatchError),
    /// Pixel geometry validation failed while normalizing a captured frame.
    #[error("KWin frame geometry is invalid: {0}")]
    Geometry(#[from] GeometryError),
    /// An internal invariant was violated (arithmetic overflow, a settled
    /// state that was not settled).
    #[error("internal KWin ScreenShot2 error: {0}")]
    Internal(&'static str),
}

impl From<zbus::Error> for KwinError {
    fn from(source: zbus::Error) -> Self {
        Self::Dbus { source }
    }
}

impl From<KwinError> for CaptureError {
    fn from(error: KwinError) -> Self {
        match error {
            KwinError::Timeout { .. } => CaptureError::Timeout {
                backend: BackendKind::KwinScreenShot2,
            },
            other => CaptureError::Backend {
                backend: BackendKind::KwinScreenShot2,
                source: Box::new(other),
            },
        }
    }
}

impl BackendError for KwinError {
    const KIND: BackendKind = BackendKind::KwinScreenShot2;

    fn timeout(timeout: Duration) -> Self {
        Self::Timeout { timeout }
    }

    fn internal(message: &'static str) -> Self {
        Self::Internal(message)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn backend_kind_is_kwin_screenshot2() {
        assert_eq!(
            <KwinError as BackendError>::KIND,
            BackendKind::KwinScreenShot2
        );
    }

    #[test]
    fn timeout_lifts_to_capture_timeout_with_the_backend_tag() {
        let err = CaptureError::from(KwinError::timeout(Duration::from_secs(15)));
        assert!(matches!(
            err,
            CaptureError::Timeout {
                backend: BackendKind::KwinScreenShot2
            }
        ));
    }

    #[test]
    fn decode_failures_lift_to_backend_errors_keeping_the_source() {
        let decode = KwinError::from(DecodeError::UnknownFormat { format: 999 });
        match CaptureError::from(decode) {
            CaptureError::Backend { backend, source } => {
                assert_eq!(backend, BackendKind::KwinScreenShot2);
                let kwin = source.downcast_ref::<KwinError>();
                assert!(matches!(
                    kwin,
                    Some(KwinError::Decode(DecodeError::UnknownFormat {
                        format: 999
                    }))
                ));
            }
            other => panic!("expected Backend, got {other:?}"),
        }
    }

    #[test]
    fn cancelled_display_names_the_cancellation() {
        assert_eq!(
            KwinError::Cancelled.to_string(),
            "the KWin screenshot was cancelled"
        );
    }
}
