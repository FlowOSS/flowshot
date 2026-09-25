//! Typed errors for the two `org.freedesktop.portal` capture backends.
//!
//! Both portal backends share one transport stack (ashpd `D-Bus` calls, a
//! one-shot Wayland connection for output geometry, and - for `ScreenCast` -
//! `PipeWire`), so they share one failure vocabulary
//! ([`PortalErrorKind`]). The backend identity lives in the two thin
//! newtypes ([`PortalScreenshotError`], [`PortalScreenCastError`]): each
//! implements [`BackendError`] with its own honest [`BackendKind`], so the
//! shared worker bridge and deadline machinery tag failures correctly
//! without duplicating the variant table twice.

use std::time::Duration;

use flowshot_capture::{BackendKind, CaptureError};
use flowshot_core::geometry::GeometryError;
use thiserror::Error;

use crate::error::{BackendError, ConnectError};

/// How a portal interaction ended without producing a result.
///
/// Mirrors the `org.freedesktop.portal.Request` response statuses: `1`
/// (user cancelled) maps to [`PortalDenial::Cancelled`], `2` (ended another
/// way) to [`PortalDenial::Other`]. Both surface as
/// [`PermissionResult::Denied`](flowshot_capture::PermissionResult::Denied)
/// at the backend boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortalDenial {
    /// The user dismissed the request (response status 1).
    Cancelled,
    /// The interaction ended in some other way (response status 2).
    Other,
}

/// The shared failure vocabulary of both portal backends.
///
/// [`CaptureError`] conversion happens on the backend newtypes, which tag
/// each failure with their own [`BackendKind`].
#[derive(Debug, Error)]
pub enum PortalErrorKind {
    /// The one-shot Wayland connection used for output geometry could not be
    /// established.
    #[error("portal capture could not connect to the Wayland session: {0}")]
    Connect(#[from] ConnectError),
    /// A portal phase did not complete within its deadline. On a desktop
    /// with an interactive picker this also covers a picker the user never
    /// answered: the backend never waits longer than the budget.
    #[error("the portal did not complete within {timeout:?}")]
    Timeout {
        /// The phase budget that expired.
        timeout: Duration,
    },
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
    /// The portal request was dismissed or ended without a result (response
    /// status 1 or 2); capture must not proceed and the caller surfaces a
    /// notification.
    #[error("the portal request was denied ({reason:?})")]
    Denied {
        /// Which unsuccessful response status the portal sent.
        reason: PortalDenial,
    },
    /// The compositor delivered a permission-denial black frame (the
    /// `Hyprland` enforce-permissions denial image, reachable through the
    /// `grim`-based portal screenshot path).
    #[error("screen capture permission was denied by the compositor")]
    PermissionDenied,
    /// A portal `D-Bus` call failed (transport, name resolution, or a
    /// portal-side method error).
    #[error("portal D-Bus call failed: {source}")]
    Dbus {
        /// The underlying ashpd error.
        #[source]
        source: ashpd::Error,
    },
    /// The Screenshot portal returned a URI that is not a readable `file:`
    /// URI.
    #[error("the portal returned a non-file screenshot URI: {uri}")]
    FileUri {
        /// The URI the portal returned.
        uri: String,
    },
    /// The screenshot file could not be decoded as an image.
    #[error("the portal screenshot file could not be decoded: {source}")]
    Decode {
        /// The underlying decoder error.
        #[source]
        source: image::ImageError,
    },
    /// An operating-system call failed (temp file read/delete, runtime
    /// setup).
    #[error("an OS call failed during portal capture: {0}")]
    Io(#[from] std::io::Error),
    /// The portal composite's dimensions match neither the layout's
    /// physical nor its logical bounding box, so its pixel space cannot be
    /// determined and per-output crops would be wrong.
    #[error(
        "portal screenshot {image:?} matches neither the physical {physical:?} nor the \
         logical {logical:?} layout size"
    )]
    UnknownPixelSpace {
        /// The decoded image dimensions.
        image: (u32, u32),
        /// The layout's physical bounding-box dimensions.
        physical: (i32, i32),
        /// The layout's logical bounding-box dimensions.
        logical: (i32, i32),
    },
    /// The portal composite is in LOGICAL pixel space while the layout has
    /// outputs at scale != 1: honoring the physical-first rule would need a
    /// resample this backend refuses to do (v1 limitation, hardware-pending).
    #[error(
        "portal screenshot {image:?} is in logical pixel space, unsupported for HiDPI \
         layouts in v1 (physical-first rule forbids resampling)"
    )]
    LogicalSpaceUnsupported {
        /// The decoded image dimensions.
        image: (u32, u32),
    },
    /// A `PipeWire` operation failed (loop, context, core, or stream
    /// setup).
    #[error("PipeWire failed during portal screencast: {source}")]
    PipeWire {
        /// The underlying pipewire error.
        #[source]
        source: pipewire::Error,
    },
    /// The portal started the screencast session but returned zero streams.
    #[error("the portal screencast session returned no streams")]
    NoStreams,
    /// A `PipeWire` stream entered the error state before delivering a
    /// frame.
    #[error("the PipeWire stream for node {node_id} failed")]
    StreamFailed {
        /// The portal-assigned node id of the failed stream.
        node_id: u32,
    },
    /// A captured stream could not be matched to any output (no mapping id,
    /// no position match, and an ambiguous size).
    #[error("screencast stream {node_id} ({frame:?}) matches no output")]
    StreamUnmapped {
        /// The portal-assigned node id of the stream.
        node_id: u32,
        /// The captured frame dimensions of the stream.
        frame: (u32, u32),
    },
    /// The session budget was exhausted without capturing every output
    /// (implementations like `XDPH` share ONE source per session, so the
    /// backend opens one session per output; a picker that keeps selecting
    /// already-captured outputs lands here).
    #[error("portal screencast did not capture outputs: {missing:?}")]
    OutputsUncaptured {
        /// The connector names still missing frames.
        missing: Vec<String>,
    },
    /// The negotiated `PipeWire` video format is not one of the v1
    /// 32-bpp raw formats (`BGRx`, `BGRA`, `RGBx`, `RGBA`).
    #[error("unsupported PipeWire video format {format}")]
    UnsupportedFormat {
        /// The `Debug` rendering of the negotiated `spa` video format.
        format: String,
    },
    /// The stream delivered a `dma-buf` buffer. v1 is shared-memory only
    /// (crate-wide policy, matching the ICC and screencopy backends); the
    /// consumer-side format negotiation requests raw formats without
    /// modifiers, so producers allocate `MemFd` buffers - this fires only
    /// when a producer ignores the negotiation.
    #[error("the PipeWire stream delivered a dma-buf buffer; v1 captures shared memory only")]
    DmabufUnsupported,
    /// The captured stream buffer size matches neither the output's native
    /// nor its post-transform physical size.
    #[error("captured stream buffer {reported:?} does not match output size {expected:?}")]
    BufferSizeMismatch {
        /// The size the stream delivered.
        reported: (u32, u32),
        /// The post-transform size enumeration predicted.
        expected: (i32, i32),
    },
    /// A delivered buffer was empty, unmapped, or shorter than its geometry
    /// claims.
    #[error("the portal delivered an incomplete frame")]
    IncompleteFrame,
    /// Pixel geometry validation failed while normalizing a captured frame.
    #[error("portal frame geometry is invalid: {0}")]
    Geometry(#[from] GeometryError),
    /// The Wayland connection broke while output geometry was collected.
    #[error("the Wayland transport failed during portal capture: {0}")]
    Transport(#[from] wayland_client::backend::WaylandError),
    /// The compositor sent a protocol error while output geometry was
    /// collected.
    #[error("Wayland protocol error during portal capture: {0}")]
    Protocol(#[from] wayland_client::DispatchError),
    /// An internal invariant was violated (arithmetic overflow, a settled
    /// state that was not settled).
    #[error("internal portal error: {0}")]
    Internal(&'static str),
}

/// Generates one portal backend error newtype over [`PortalErrorKind`]:
/// the [`BackendError`] implementation with the backend's own
/// [`BackendKind`], the infrastructure `From` conversions the shared
/// deadline/worker machinery needs, and the [`CaptureError`] lift.
macro_rules! portal_backend_error {
    ($name:ident, $kind:ident, $doc:literal) => {
        #[derive(Debug, Error)]
        #[error(transparent)]
        #[doc = $doc]
        pub struct $name(#[from] pub(crate) PortalErrorKind);

        impl $name {
            /// The shared portal failure this error carries.
            #[must_use]
            pub fn kind(&self) -> &PortalErrorKind {
                &self.0
            }

            /// Returns `true` when the failure is a permission denial: the
            /// portal interaction was dismissed (response status 1/2) or the
            /// compositor delivered a denial frame. Callers map this onto
            /// [`PermissionResult::Denied`](flowshot_capture::PermissionResult::Denied).
            #[must_use]
            pub fn is_denied(&self) -> bool {
                matches!(
                    self.0,
                    PortalErrorKind::Denied { .. } | PortalErrorKind::PermissionDenied
                )
            }
        }

        impl BackendError for $name {
            const KIND: BackendKind = BackendKind::$kind;

            fn timeout(timeout: Duration) -> Self {
                Self(PortalErrorKind::Timeout { timeout })
            }

            fn internal(message: &'static str) -> Self {
                Self(PortalErrorKind::Internal(message))
            }
        }

        impl From<ConnectError> for $name {
            fn from(source: ConnectError) -> Self {
                Self(PortalErrorKind::from(source))
            }
        }

        impl From<std::io::Error> for $name {
            fn from(source: std::io::Error) -> Self {
                Self(PortalErrorKind::from(source))
            }
        }

        impl From<wayland_client::backend::WaylandError> for $name {
            fn from(source: wayland_client::backend::WaylandError) -> Self {
                Self(PortalErrorKind::from(source))
            }
        }

        impl From<wayland_client::DispatchError> for $name {
            fn from(source: wayland_client::DispatchError) -> Self {
                Self(PortalErrorKind::from(source))
            }
        }

        impl From<$name> for CaptureError {
            fn from(error: $name) -> Self {
                match error.0 {
                    PortalErrorKind::Timeout { .. } => CaptureError::Timeout {
                        backend: BackendKind::$kind,
                    },
                    other => CaptureError::Backend {
                        backend: BackendKind::$kind,
                        source: Box::new(other),
                    },
                }
            }
        }
    };
}

portal_backend_error!(
    PortalScreenshotError,
    PortalScreenshot,
    "Why an `org.freedesktop.portal.Screenshot` capture failed.\n\n\
     A thin newtype over [`PortalErrorKind`] tagging every failure with\n\
     [`BackendKind::PortalScreenshot`] when lifted into a [`CaptureError`]:\n\
     `Timeout` maps onto [`CaptureError::Timeout`], everything else onto\n\
     [`CaptureError::Backend`]."
);

portal_backend_error!(
    PortalScreenCastError,
    PortalScreenCast,
    "Why an `org.freedesktop.portal.ScreenCast` capture failed.\n\n\
     A thin newtype over [`PortalErrorKind`] tagging every failure with\n\
     [`BackendKind::PortalScreenCast`] when lifted into a [`CaptureError`]:\n\
     `Timeout` maps onto [`CaptureError::Timeout`], everything else onto\n\
     [`CaptureError::Backend`]."
);

/// The error contract the shared portal plumbing needs beyond
/// [`BackendError`]: the one-shot output collection can also fail at the
/// Wayland connect step.
pub(crate) trait PortalBackendError: BackendError + From<ConnectError> {}

impl PortalBackendError for PortalScreenshotError {}
impl PortalBackendError for PortalScreenCastError {}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn newtypes_report_their_own_backend_kind() {
        assert_eq!(
            <PortalScreenshotError as BackendError>::KIND,
            BackendKind::PortalScreenshot
        );
        assert_eq!(
            <PortalScreenCastError as BackendError>::KIND,
            BackendKind::PortalScreenCast
        );
    }

    #[test]
    fn timeout_lifts_to_capture_timeout_with_the_backend_tag() {
        let err = CaptureError::from(PortalScreenCastError::timeout(Duration::from_secs(15)));
        assert!(matches!(
            err,
            CaptureError::Timeout {
                backend: BackendKind::PortalScreenCast
            }
        ));
        let err = CaptureError::from(PortalScreenshotError::timeout(Duration::from_secs(15)));
        assert!(matches!(
            err,
            CaptureError::Timeout {
                backend: BackendKind::PortalScreenshot
            }
        ));
    }

    #[test]
    fn other_failures_lift_to_backend_errors_keeping_the_source() {
        let err = CaptureError::from(PortalScreenshotError::from(PortalErrorKind::NoStreams));
        match err {
            CaptureError::Backend { backend, source } => {
                assert_eq!(backend, BackendKind::PortalScreenshot);
                assert!(source.downcast_ref::<PortalErrorKind>().is_some());
            }
            other => panic!("expected Backend, got {other:?}"),
        }
    }

    #[test]
    fn is_denied_covers_both_denial_families() {
        let dismissed = PortalScreenCastError::from(PortalErrorKind::Denied {
            reason: PortalDenial::Cancelled,
        });
        let blackframe = PortalScreenshotError::from(PortalErrorKind::PermissionDenied);
        let timeout = PortalScreenshotError::timeout(Duration::from_secs(1));
        assert!(dismissed.is_denied());
        assert!(blackframe.is_denied());
        assert!(!timeout.is_denied());
    }

    #[test]
    fn display_forwards_to_the_shared_kind() {
        let err = PortalScreenshotError::from(PortalErrorKind::NoStreams);
        assert_eq!(
            err.to_string(),
            "the portal screencast session returned no streams"
        );
    }
}
