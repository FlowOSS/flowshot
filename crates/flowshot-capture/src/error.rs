//! Typed errors for the capture subsystem.
//!
//! Every failure crossing a [`CaptureBackend`](crate::CaptureBackend) boundary
//! is a variant of [`CaptureError`]; backends wrap implementation-specific
//! causes in the `source` field rather than inventing stringly errors.

use std::fmt;

use flowshot_core::geometry::LogicalRect;
use thiserror::Error;

use crate::kind::BackendKind;

/// The error type of all capture operations.
#[derive(Debug, Error)]
pub enum CaptureError {
    /// Negotiation found no usable backend for the probed session.
    ///
    /// The `Display` message names every missing protocol so users and logs
    /// say exactly what the session lacks.
    #[error(
        "no capture backend available; missing protocols: {}",
        ProtocolList(missing)
    )]
    NoBackendAvailable {
        /// The backends that were considered and found missing, in ladder
        /// order (or the single forced backend when an override failed).
        missing: Vec<BackendKind>,
    },
    /// A backend did not deliver a frame within its deadline.
    #[error("capture timed out on backend {backend}")]
    Timeout {
        /// The backend that ran out of time.
        backend: BackendKind,
    },
    /// A captured or embedded image could not be decoded into a frame buffer.
    #[error("frame decode failed on backend {backend}")]
    Decode {
        /// The backend whose image failed to decode.
        backend: BackendKind,
        /// The underlying decoder error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// The requested region does not intersect any output of the layout, or
    /// the intersection rounds to zero physical pixels.
    #[error(
        "region ({}, {} {}x{}) does not intersect any output",
        region.x.0,
        region.y.0,
        region.width.0,
        region.height.0
    )]
    RegionOutsideLayout {
        /// The region that missed every output.
        region: LogicalRect,
    },
    /// A backend-specific failure with the originating cause attached.
    #[error("capture backend {backend} failed")]
    Backend {
        /// The backend that failed.
        backend: BackendKind,
        /// The underlying protocol or platform error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

/// Renders backend kinds as their protocol names inside error messages.
struct ProtocolList<'a>(&'a [BackendKind]);

impl fmt::Display for ProtocolList<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, kind) in self.0.iter().enumerate() {
            if index > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{kind}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn no_backend_message_names_every_missing_protocol() {
        let err = CaptureError::NoBackendAvailable {
            missing: vec![
                BackendKind::ExtImageCopyCapture,
                BackendKind::WlrScreencopy,
                BackendKind::KwinScreenShot2,
                BackendKind::PortalScreenCast,
                BackendKind::PortalScreenshot,
            ],
        };
        let message = err.to_string();
        assert!(message.contains("ext-image-copy-capture-v1"), "{message}");
        assert!(message.contains("zwlr-screencopy-v1"), "{message}");
        assert!(message.contains("org.kde.KWin.ScreenShot2"), "{message}");
        assert!(
            message.contains("org.freedesktop.portal.ScreenCast"),
            "{message}"
        );
        assert!(
            message.contains("org.freedesktop.portal.Screenshot"),
            "{message}"
        );
    }

    #[test]
    fn backend_error_exposes_its_source() {
        let err = CaptureError::Backend {
            backend: BackendKind::WlrScreencopy,
            source: "protocol exploded".to_owned().into(),
        };
        assert!(std::error::Error::source(&err).is_some());
        assert_eq!(err.to_string(), "capture backend zwlr-screencopy-v1 failed");
    }
}
