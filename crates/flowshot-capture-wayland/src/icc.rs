//! The `ext-image-copy-capture-v1` frame backend
//! ([`BackendKind::ExtImageCopyCapture`]).
//!
//! # Execution model
//!
//! Every operation runs on a short-lived worker thread holding a ONE-SHOT
//! Wayland connection (`grim`-style): connect, collect the session, run the
//! per-output capture chain from [`run`], tear the connection down. The
//! async trait methods bridge to that thread through a `futures` oneshot
//! channel, so they never block the caller's executor and stay
//! runtime-agnostic (no tokio dependency). One-shot connections make
//! teardown unconditional: whatever the outcome, closing the connection
//! releases every capture session, frame, pool, and buffer compositor-side,
//! so a failed or cancelled capture can never leak sessions.
//!
//! The long-lived [`CaptureThread`] remains the probing/hotplug facility;
//! this backend deliberately does not park a thread between captures.
//!
//! # Permissions
//!
//! Wayland has no pre-capture permission API; compositors with a permission
//! system (e.g. `Hyprland` with `ecosystem:enforce_permissions`) decide at
//! capture time. [`IccBackend::request_permission`] therefore runs a probe
//! capture of the first output and classifies the result: a denial black
//! frame ([`denial`]) maps to [`PermissionResult::Denied`], real content to
//! [`PermissionResult::Granted`], and any failure (including the 10s
//! timeout a PENDING permission popup provokes) to `Denied` with a warning -
//! the contract is "never hang", and denial is the safe side. Regular
//! captures enforce the same classification: a denial frame surfaces as
//! [`IccError::PermissionDenied`] inside [`CaptureError::Backend`] instead
//! of a black screenshot.
//!
//! [`CaptureThread`]: crate::CaptureThread

pub(crate) mod dispatch;
pub(crate) mod protocol;
mod run;
pub(crate) mod shm;
pub(crate) mod wait;

use async_trait::async_trait;
use flowshot_capture::{
    BackendKind, CaptureBackend, CaptureError, CaptureOpts, CursorStream, Frame, PermissionResult,
};
use flowshot_core::geometry::{LogicalRect, OutputInfo};

use crate::error::IccError;
use crate::worker::spawn_worker;

/// The `ext-image-copy-capture-v1` capture backend.
///
/// Stateless: each operation opens its own one-shot compositor connection
/// on a worker thread (see the module docs). Cheap to construct, `Send +
/// Sync`, and safe to drop at any point.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IccBackend;

impl IccBackend {
    /// Creates the backend.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Captures the single output with this connector name.
    ///
    /// Used by the QA harnesses (forced bad output name -> typed error;
    /// `HiDPI` spike -> per-output buffer assertion) and available to
    /// callers that target one output.
    ///
    /// # Errors
    ///
    /// [`CaptureError::Backend`] wrapping
    /// [`IccError::OutputNotFound`] (listing the available connectors) when
    /// the session does not advertise `connector`, plus every error of
    /// [`CaptureBackend::capture_outputs`] for the capture itself.
    pub async fn capture_output_named(
        &self,
        connector: &str,
        opts: CaptureOpts,
    ) -> Result<Frame, CaptureError> {
        let name = connector.to_owned();
        let captured = spawn_worker("flowshot-icc-named", move || {
            run::capture_run(opts, &run::Selection::Named(name))
        })
        .await?;
        let frame = captured
            .frames
            .into_iter()
            .next()
            .ok_or_else(|| CaptureError::Backend {
                backend: BackendKind::ExtImageCopyCapture,
                source: IccError::Internal("a named capture returned no frame").into(),
            })?;
        Ok(frame)
    }
}

#[async_trait]
impl CaptureBackend for IccBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::ExtImageCopyCapture
    }

    async fn outputs(&self) -> Result<Vec<OutputInfo>, CaptureError> {
        spawn_worker("flowshot-icc-outputs", run::outputs_run).await
    }

    async fn capture_outputs(&self, opts: CaptureOpts) -> Result<Vec<Frame>, CaptureError> {
        let captured = spawn_worker("flowshot-icc-capture", move || {
            run::capture_run(opts, &run::Selection::All)
        })
        .await?;
        Ok(captured.frames)
    }

    async fn capture_region(&self, region: LogicalRect) -> Result<Frame, CaptureError> {
        // Region captures never paint the cursor: the stitched composite is
        // content for editors and savers, matching grim's region behavior
        // and the MockBackend contract (cursor painting is a per-output
        // capture option, todo 8 owns cursor compositing).
        let captured = spawn_worker("flowshot-icc-region", || {
            run::capture_run(CaptureOpts::new(false), &run::Selection::All)
        })
        .await?;
        captured.stitch(BackendKind::ExtImageCopyCapture, region)
    }

    fn cursor_events(&self) -> Option<CursorStream> {
        // Bridges ext-image-copy-capture-v1 pointer-cursor sessions (one per
        // output) onto the shared stream from a dedicated event-driven worker;
        // None only when the worker thread cannot be spawned (a degradation,
        // never a capture failure).
        crate::cursor::stream::spawn_cursor_stream()
    }

    async fn request_permission(&self) -> PermissionResult {
        match spawn_worker("flowshot-icc-permission", || {
            run::capture_run(CaptureOpts::new(false), &run::Selection::First)
        })
        .await
        {
            Ok(_captured) => PermissionResult::Granted,
            Err(CaptureError::Backend { source, .. })
                if matches!(
                    source.downcast_ref::<IccError>(),
                    Some(IccError::PermissionDenied)
                ) =>
            {
                PermissionResult::Denied
            }
            Err(error) => {
                // Never hang, never guess "granted": a failed probe (no
                // outputs, timeout behind a pending permission popup,
                // protocol failure) reports the safe side and logs the real
                // cause.
                tracing::warn!(%error, "permission probe capture failed; reporting denied");
                PermissionResult::Denied
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use flowshot_capture::{CapabilityProbe, DesktopEnv, negotiate};

    use crate::globals::{Global, ProtocolGlobals};

    use super::*;

    #[test]
    fn backend_reports_its_kind() {
        assert_eq!(IccBackend::new().kind(), BackendKind::ExtImageCopyCapture);
    }

    #[test]
    fn icc_masked_probe_negotiates_down_to_screencopy() {
        // A session advertising everything EXCEPT the two ICC managers:
        // negotiation must fall to wlr-screencopy as the first rung.
        let globals = vec![
            Global::new(1, "zwlr_screencopy_manager_v1", 3),
            Global::new(2, "zxdg_output_manager_v1", 3),
            Global::new(3, "wl_shm", 1),
            Global::new(4, "ext_foreign_toplevel_image_capture_source_manager_v1", 1),
        ];
        let probe: CapabilityProbe =
            ProtocolGlobals::from_globals(&globals).to_capability_probe(DesktopEnv::Hyprland);
        assert!(!probe.supports(BackendKind::ExtImageCopyCapture));
        let ladder = negotiate(&probe, None).unwrap();
        assert_eq!(ladder.first(), Some(&BackendKind::WlrScreencopy));
    }

    #[test]
    fn forced_icc_on_a_masked_probe_fails_fast() {
        let globals = vec![Global::new(1, "zwlr_screencopy_manager_v1", 3)];
        let probe =
            ProtocolGlobals::from_globals(&globals).to_capability_probe(DesktopEnv::Hyprland);
        let err = negotiate(&probe, Some(BackendKind::ExtImageCopyCapture)).unwrap_err();
        assert!(matches!(err, CaptureError::NoBackendAvailable { .. }));
        assert!(err.to_string().contains("ext-image-copy-capture-v1"));
    }
}
