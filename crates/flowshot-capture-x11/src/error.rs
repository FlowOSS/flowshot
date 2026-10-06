//! Typed errors for connecting to, probing, and capturing on an X11 session.
//!
//! Mirrors the `flowshot-capture-wayland` error family: every failure is a
//! typed variant, the connect failure carries an environment-derived
//! remediation hint, and the [`flowshot_capture::CaptureError`] conversion
//! maps [`X11Error::Timeout`] onto [`CaptureError::Timeout`] and everything
//! else onto [`CaptureError::Backend`] tagged [`BackendKind::X11`].

use std::time::Duration;

use flowshot_capture::{BackendKind, CaptureError};
use flowshot_core::geometry::GeometryError;
use thiserror::Error;

/// Why an X11 session operation failed.
#[derive(Debug, Error)]
pub enum X11Error {
    /// The X server socket could not be connected. The `hint` explains the
    /// most likely cause based on the process environment.
    #[error("could not connect to the X server: {source} (hint: {hint})")]
    Connect {
        /// The underlying connection error from `x11rb`.
        source: x11rb::errors::ConnectError,
        /// Human-readable remediation hint derived from the environment.
        hint: String,
    },
    /// The X server rejected a request or the connection broke while a
    /// reply was in flight.
    #[error("X11 request failed: {0}")]
    Protocol(#[from] x11rb::errors::ReplyError),
    /// An X11 extension required for the operation is not present on the
    /// server.
    #[error("the X server does not provide {name}")]
    MissingExtension {
        /// The name of the missing extension (e.g. `RANDR`).
        name: &'static str,
    },
    /// The server's pixel depth is not one this backend can capture
    /// (24-bit `TrueColor` pixels are required).
    #[error("unsupported X11 pixel depth {depth}: capture needs 24-bit pixels")]
    UnsupportedDepth {
        /// The depth the server reported, in bits per pixel.
        depth: u8,
    },
    /// The server negotiates MSB-first image byte order, which the Phase A
    /// pixel-format mapping (little-endian shared-memory conventions,
    /// [`FrameFormat::Xrgb8888`]) does not cover.
    ///
    /// [`FrameFormat::Xrgb8888`]: flowshot_capture::FrameFormat::Xrgb8888
    #[error("unsupported X11 image byte order: capture maps LSB-first (little-endian) pixels only")]
    UnsupportedByteOrder,
    /// An image reply carried a different number of bytes than the requested
    /// geometry requires (a server violating the `GetImage` contract).
    #[error("X11 image reply carries {actual} bytes; the requested geometry needs {expected}")]
    ImageSizeMismatch {
        /// The byte count the requested geometry requires.
        expected: usize,
        /// The byte count the server actually reported or delivered.
        actual: usize,
    },
    /// An operating-system call failed (anonymous file creation, sizing, or
    /// pixel readback for the `MIT-SHM` fast path).
    #[error("an OS call failed during X11 capture: {0}")]
    Io(#[from] std::io::Error),
    /// The X server did not complete an operation within its deadline.
    #[error("the X11 operation did not complete within {timeout:?}")]
    Timeout {
        /// The deadline that expired.
        timeout: Duration,
    },
    /// Output geometry validation failed while mapping RANDR data onto the
    /// shared geometry types.
    #[error("X11 output geometry is invalid: {0}")]
    Geometry(#[from] GeometryError),
    /// An internal invariant was violated (a server-reported index that
    /// cannot occur on a healthy connection).
    #[error("internal X11 error: {0}")]
    Internal(&'static str),
}

impl From<x11rb::errors::ConnectionError> for X11Error {
    fn from(error: x11rb::errors::ConnectionError) -> Self {
        Self::Protocol(x11rb::errors::ReplyError::from(error))
    }
}

impl From<X11Error> for CaptureError {
    fn from(error: X11Error) -> Self {
        match error {
            X11Error::Timeout { .. } => CaptureError::Timeout {
                backend: BackendKind::X11,
            },
            other => CaptureError::Backend {
                backend: BackendKind::X11,
                source: Box::new(other),
            },
        }
    }
}

/// Wraps an `x11rb` connect failure with an environment-based hint.
pub(crate) fn connect_error(source: x11rb::errors::ConnectError) -> X11Error {
    let display = std::env::var("DISPLAY").ok();
    let hint = connect_hint(display.as_deref().filter(|value| !value.is_empty()));
    X11Error::Connect { source, hint }
}

/// Builds the remediation hint for a failed X connection.
///
/// Pure function of the `DISPLAY` value `x11rb` consults, so the message
/// logic is testable without touching the process environment.
fn connect_hint(display: Option<&str>) -> String {
    match display {
        None => "DISPLAY is not set; are you running inside an X11 session? \
                 (under a Wayland session without XWayland, or on a TTY, X11 capture \
                 is unavailable)"
            .to_owned(),
        Some(display) => format!(
            "DISPLAY={display} does not name a reachable X server; \
             is the server running and its socket accessible to this user?"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use x11rb::errors::{ConnectError as X11rbConnectError, ConnectionError};

    #[test]
    fn hint_names_missing_display() {
        let hint = connect_hint(None);
        assert!(hint.contains("DISPLAY is not set"), "{hint}");
    }

    #[test]
    fn hint_echoes_the_display_value() {
        let hint = connect_hint(Some(":1"));
        assert!(hint.contains("DISPLAY=:1"), "{hint}");
        assert!(hint.contains("reachable X server"), "{hint}");
    }

    #[test]
    fn connect_error_display_includes_hint_and_source() {
        let err = X11Error::Connect {
            source: X11rbConnectError::UnknownError,
            hint: "check the session".to_owned(),
        };
        let text = err.to_string();
        assert!(text.contains("check the session"), "{text}");
        assert!(text.contains("Unknown connection error"), "{text}");
    }

    #[test]
    fn missing_extension_names_the_extension() {
        let err = X11Error::MissingExtension { name: "RANDR" };
        assert!(err.to_string().contains("RANDR"), "{err}");
    }

    #[test]
    fn timeout_maps_to_capture_timeout_tagged_x11() {
        let err = X11Error::Timeout {
            timeout: Duration::from_secs(10),
        };
        assert!(matches!(
            CaptureError::from(err),
            CaptureError::Timeout {
                backend: BackendKind::X11
            }
        ));
    }

    #[test]
    fn protocol_errors_map_to_backend_tagged_x11() {
        let err = X11Error::from(ConnectionError::UnknownError);
        match CaptureError::from(err) {
            CaptureError::Backend { backend, source } => {
                assert_eq!(backend, BackendKind::X11);
                assert!(!source.to_string().is_empty(), "{source}");
            }
            other => panic!("expected CaptureError::Backend, got {other:?}"),
        }
    }
}
