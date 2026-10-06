//! The X11 [`CaptureBackend`]: [`X11Backend`], the negotiation ladder's X11
//! rung (`BackendKind::X11`).
//!
//! # Execution model
//!
//! The Wayland sibling's worker pattern: every operation runs its blocking
//! `x11rb` chain on a short-lived worker thread holding a ONE-SHOT connection
//! ([`spawn_worker`]), bridged into a runtime-agnostic future. Closing the
//! connection is the universal cleanup, so a failed or cancelled capture never
//! leaks server-side resources. The [`X11Caps`] probed at construction travel
//! into each worker by value (they are stable for the server's lifetime), so
//! the per-capture transport decision (`MIT-SHM` fast path vs plain
//! `GetImage`) costs no extra round-trips.
//!
//! # Phase A degradations (documented, never failures)
//!
//! - [`cursor_events`](CaptureBackend::cursor_events) is `None`: X11 offers
//!   no cursor *stream* to a client - only the one-shot reads
//!   ([`X11Backend::cursor_pos`], XFIXES `GetCursorImage` per capture). Phase
//!   B may add an XFIXES `CursorNotify` poll stream.
//! - Region captures never paint the cursor (the stitched composite is
//!   content for editors and savers - the Wayland backends' and
//!   [`MockBackend`](flowshot_capture::MockBackend)'s contract).
//! - [`request_permission`](CaptureBackend::request_permission) is
//!   [`PermissionResult::NotRequired`]: X11 has no capture permission model
//!   (plan decision #9).
//!
//! # Consistency
//!
//! Sequential per-output `GetImage` reads are not atomic across outputs:
//! fast-moving content can skew between monitors in a stitched region
//! (`grim`/`scrot`-equivalent behavior, documented in the stitcher).

use async_trait::async_trait;
use flowshot_capture::{
    BackendKind, CaptureBackend, CaptureError, CaptureOpts, CursorStream, Frame, PermissionResult,
};
use flowshot_core::geometry::{LogicalPoint, LogicalRect, OutputInfo};

use crate::capture::capture_run;
use crate::connect::X11Connection;
use crate::cursor;
use crate::error::X11Error;
use crate::output;
use crate::probe::{X11Caps, query_caps};
use crate::worker::spawn_worker;

/// The X11 capture backend.
///
/// Constructed once ([`X11Backend::connect`]) with the session's probed
/// capabilities; every operation then opens its own one-shot connection on a
/// worker thread (see the module docs). Cheap to hold, `Send + Sync`, and
/// safe to drop at any point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct X11Backend {
    caps: X11Caps,
    display: String,
}

impl X11Backend {
    /// Connects to the `DISPLAY` session, probes the capture capabilities,
    /// and records the display string for diagnostics.
    ///
    /// # Errors
    ///
    /// Propagates the [`X11Error`] of the connect and capability probe
    /// (a missing RANDR is fatal - output enumeration needs it).
    pub fn connect() -> Result<Self, X11Error> {
        let conn = X11Connection::connect()?;
        let caps = query_caps(conn.conn())?;
        let display = std::env::var("DISPLAY").unwrap_or_default();
        Ok(Self { caps, display })
    }

    /// The capabilities probed at construction.
    #[must_use]
    pub const fn caps(&self) -> X11Caps {
        self.caps
    }

    /// The `DISPLAY` string this backend probed (empty when the environment
    /// lost it between probe and read - diagnostics only).
    #[must_use]
    pub fn display(&self) -> &str {
        &self.display
    }

    /// Returns a copy with the `MIT-SHM` fast path disabled, forcing plain
    /// `GetImage` captures (diagnostics and QA cross-checks).
    #[must_use]
    pub fn without_shm(mut self) -> Self {
        self.caps.shm = None;
        self
    }

    /// The one-shot cursor position in global logical layout space
    /// (`XQueryPointer`), for cursor-aware preselect.
    ///
    /// `Ok(None)` when the pointer sits inside no output (a layout gap or
    /// another screen of a multi-screen display) - callers degrade, they
    /// never fail on a missing cursor.
    ///
    /// # Errors
    ///
    /// [`CaptureError::Backend`] tagged [`BackendKind::X11`] when the
    /// `QueryPointer` read or the RANDR enumeration fails.
    pub async fn cursor_pos(&self) -> Result<Option<LogicalPoint>, CaptureError> {
        spawn_worker("flowshot-x11-cursor", || {
            let conn = X11Connection::connect()?;
            cursor::cursor_pos(&conn)
        })
        .await
    }
}

#[async_trait]
impl CaptureBackend for X11Backend {
    fn kind(&self) -> BackendKind {
        BackendKind::X11
    }

    async fn outputs(&self) -> Result<Vec<OutputInfo>, CaptureError> {
        spawn_worker("flowshot-x11-outputs", || {
            let conn = X11Connection::connect()?;
            output::outputs(&conn)
        })
        .await
    }

    async fn capture_outputs(&self, opts: CaptureOpts) -> Result<Vec<Frame>, CaptureError> {
        let caps = self.caps;
        let captured = spawn_worker("flowshot-x11-capture", move || {
            let conn = X11Connection::connect()?;
            capture_run(&conn, caps, opts)
        })
        .await?;
        Ok(captured.frames)
    }

    async fn capture_region(&self, region: LogicalRect) -> Result<Frame, CaptureError> {
        // Region captures never paint the cursor: the stitched composite is
        // content for editors and savers (the shared region contract).
        let caps = self.caps;
        let captured = spawn_worker("flowshot-x11-region", move || {
            let conn = X11Connection::connect()?;
            capture_run(&conn, caps, CaptureOpts::new(false))
        })
        .await?;
        captured.stitch(region)
    }

    fn cursor_events(&self) -> Option<CursorStream> {
        // X11 has no client-visible cursor stream: Phase A reads the cursor
        // one-shot per capture (XQueryPointer + XFIXES GetCursorImage).
        // None is the contract's degradation, never a capture failure.
        None
    }

    async fn request_permission(&self) -> PermissionResult {
        // X11 has no capture permission model: any client that can connect
        // can read the root window (plan decision #9).
        PermissionResult::NotRequired
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use futures::executor::block_on;

    use super::*;
    use crate::probe::ExtensionVersion;

    fn backend() -> X11Backend {
        X11Backend {
            caps: X11Caps {
                randr: ExtensionVersion::new(1, 5),
                xfixes: Some(ExtensionVersion::new(5, 0)),
                shm: Some(ExtensionVersion::new(1, 2)),
            },
            display: ":0".to_owned(),
        }
    }

    #[test]
    fn backend_reports_the_x11_kind() {
        assert_eq!(backend().kind(), BackendKind::X11);
    }

    #[test]
    fn cursor_events_degrades_to_none() {
        assert!(backend().cursor_events().is_none());
    }

    #[test]
    fn permission_is_not_required() {
        assert_eq!(
            block_on(backend().request_permission()),
            PermissionResult::NotRequired
        );
    }

    #[test]
    fn without_shm_clears_only_the_shm_capability() {
        let plain = backend().without_shm();
        assert_eq!(plain.caps().shm, None);
        assert_eq!(plain.caps().randr, backend().caps().randr);
        assert_eq!(plain.caps().xfixes, backend().caps().xfixes);
        assert_eq!(plain.display(), ":0");
    }

    #[test]
    fn backend_is_usable_as_a_trait_object() {
        let backend: Box<dyn CaptureBackend> = Box::new(backend());
        assert_eq!(backend.kind(), BackendKind::X11);
    }
}
