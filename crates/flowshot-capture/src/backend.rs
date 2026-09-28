//! The asynchronous capture backend contract.
//!
//! Every platform implementation (compositor protocols, `D-Bus` fast paths,
//! portals, and the roadmap `X11`/Windows/`macOS` backends) implements
//! [`CaptureBackend`]. The contract is async by design: native frame delivery,
//! portal `D-Bus` round-trips, and OS pickers are all asynchronous, and a
//! synchronous capture entry point would force blocking shims on every
//! consumer while leaking the slowest transport into the UI thread.

use async_trait::async_trait;
use flowshot_core::geometry::{LogicalRect, OutputInfo};
use serde::{Deserialize, Serialize};

use crate::cursor::CursorStream;
use crate::error::CaptureError;
use crate::frame::Frame;
use crate::kind::BackendKind;

/// Options controlling a single capture pass.
///
/// Marked `#[non_exhaustive]`: capture options will grow (window capture,
/// delay, per-output selection) without breaking downstream constructors.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureOpts {
    /// Whether the cursor is composited into the captured frames.
    ///
    /// Driven by the `hide_cursor` config key: `paint_cursor = !hide_cursor`.
    pub paint_cursor: bool,
}

impl CaptureOpts {
    /// Creates options with an explicit cursor-painting choice.
    ///
    /// # Examples
    ///
    /// ```
    /// use flowshot_capture::CaptureOpts;
    ///
    /// assert!(!CaptureOpts::new(false).paint_cursor);
    /// ```
    #[must_use]
    pub const fn new(paint_cursor: bool) -> Self {
        Self { paint_cursor }
    }
}

impl Default for CaptureOpts {
    /// Paints the cursor by default, matching the shipped config default
    /// `hide_cursor = false`.
    fn default() -> Self {
        Self { paint_cursor: true }
    }
}

/// The outcome of asking the platform for capture permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PermissionResult {
    /// Permission was granted; capture may proceed.
    Granted,
    /// Permission was denied; capture must not proceed. Callers surface a
    /// notification and exit typed - they never hang waiting for a retry.
    Denied,
    /// This backend needs no permission on this platform.
    NotRequired,
}

impl PermissionResult {
    /// Returns `true` when capture may proceed (`Granted` or `NotRequired`).
    ///
    /// # Examples
    ///
    /// ```
    /// use flowshot_capture::PermissionResult;
    ///
    /// assert!(PermissionResult::NotRequired.can_capture());
    /// assert!(!PermissionResult::Denied.can_capture());
    /// ```
    #[must_use]
    pub const fn can_capture(self) -> bool {
        matches!(self, Self::Granted | Self::NotRequired)
    }
}

/// The capture backend contract shared by every platform implementation.
///
/// Implementations must be `Send + Sync` so the UI can own a
/// `Box<dyn CaptureBackend>` chosen at runtime by
/// [`negotiate()`](crate::negotiate()) and drive it from its own task.
///
/// # Contract
///
/// - Buffers are physical-pixels-first: a [`Frame`] never carries an averaged
///   scale across outputs, and consumers never rescale by a blended factor.
/// - [`CaptureBackend::request_permission`] is infallible: denial is a value,
///   not an error, so callers can branch without a hang path.
/// - A `None` from [`CaptureBackend::cursor_events`] is a degradation, never a
///   capture failure.
#[async_trait]
pub trait CaptureBackend: Send + Sync {
    /// Which backend implementation this is.
    ///
    /// # Examples
    ///
    /// ```
    /// use flowshot_capture::{BackendKind, CaptureBackend, MockBackend};
    ///
    /// let backend = MockBackend::new(BackendKind::WlrScreencopy);
    /// assert_eq!(backend.kind(), BackendKind::WlrScreencopy);
    /// ```
    fn kind(&self) -> BackendKind;

    /// Enumerates the desktop outputs visible to this backend, with logical
    /// geometry, physical size, scale, and transform.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureError::Backend`] when enumeration fails (no session
    /// connection, protocol error).
    ///
    /// # Examples
    ///
    /// ```
    /// use flowshot_capture::{BackendKind, CaptureBackend, MockBackend};
    ///
    /// let backend = MockBackend::new(BackendKind::ExtImageCopyCapture);
    /// let outputs = futures::executor::block_on(backend.outputs()).unwrap();
    /// assert_eq!(outputs.len(), 2);
    /// ```
    async fn outputs(&self) -> Result<Vec<OutputInfo>, CaptureError>;

    /// Captures every output, one [`Frame`] per output, in
    /// [`CaptureBackend::outputs`] order.
    ///
    /// Each frame's buffer is in the output's native (pre-transform)
    /// orientation; [`Frame::scale`] and [`Frame::transform`] carry the
    /// metadata needed to place it in the global logical layout.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureError::Timeout`] when a frame does not arrive within
    /// the backend's deadline, [`CaptureError::Backend`] on protocol
    /// failures, and surfaces permission denial per the backend's mapping.
    ///
    /// # Examples
    ///
    /// ```
    /// use flowshot_capture::{BackendKind, CaptureBackend, CaptureOpts, MockBackend};
    ///
    /// let backend = MockBackend::new(BackendKind::PortalScreenshot);
    /// let frames = futures::executor::block_on(
    ///     backend.capture_outputs(CaptureOpts::default()),
    /// )
    /// .unwrap();
    /// assert_eq!(frames.len(), 2);
    /// ```
    async fn capture_outputs(&self, opts: CaptureOpts) -> Result<Vec<Frame>, CaptureError>;

    /// Captures `region`, given in global logical layout space, as a single
    /// stitched [`Frame`] with [`OutputRef::Composite`](crate::OutputRef).
    ///
    /// # Errors
    ///
    /// Returns [`CaptureError::RegionOutsideLayout`] when the region does not
    /// intersect any output, plus the same transport errors as
    /// [`CaptureBackend::capture_outputs`].
    ///
    /// # Examples
    ///
    /// ```
    /// use flowshot_capture::{BackendKind, CaptureBackend, MockBackend};
    /// use flowshot_core::geometry::{Logical, LogicalRect};
    ///
    /// let backend = MockBackend::new(BackendKind::WlrScreencopy);
    /// let region = LogicalRect::new(Logical(1.0), Logical(1.0), Logical(4.0), Logical(2.0));
    /// let frame = futures::executor::block_on(backend.capture_region(region)).unwrap();
    /// assert_eq!(frame.buffer.width, 4);
    /// assert_eq!(frame.buffer.height, 2);
    /// ```
    async fn capture_region(&self, region: LogicalRect) -> Result<Frame, CaptureError>;

    /// This backend's cursor event stream, or `None` when the backend (or its
    /// permission system) cannot observe the cursor. Callers degrade
    /// gracefully on `None` - a missing cursor stream never fails a capture.
    ///
    /// # Examples
    ///
    /// ```
    /// use flowshot_capture::{BackendKind, CaptureBackend, MockBackend};
    ///
    /// let backend = MockBackend::new(BackendKind::ExtImageCopyCapture);
    /// assert!(backend.cursor_events().is_some());
    /// ```
    fn cursor_events(&self) -> Option<CursorStream>;

    /// Asks the platform for capture permission.
    ///
    /// Infallible by contract: denial arrives as
    /// [`PermissionResult::Denied`], never as an error or a hang, so callers
    /// can always surface a notification and exit typed.
    ///
    /// # Examples
    ///
    /// ```
    /// use flowshot_capture::{BackendKind, CaptureBackend, MockBackend, PermissionResult};
    ///
    /// let backend = MockBackend::new(BackendKind::WlrScreencopy);
    /// let permission = futures::executor::block_on(backend.request_permission());
    /// assert_eq!(permission, PermissionResult::NotRequired);
    /// ```
    async fn request_permission(&self) -> PermissionResult;
}
