//! The `wlr-screencopy-unstable-v1` fallback backend
//! ([`BackendKind::WlrScreencopy`]), rung 2 of the negotiation ladder.
//!
//! This is the fallback for sessions without `ext-image-copy-capture-v1`
//! (niri, wlroots < 0.19, and friends). It binds `zwlr_screencopy_manager_v1`
//! (v3) and, per output, runs `capture_output(overlay_cursor, wl_output)` ->
//! await `buffer`/`buffer_done` -> allocate a `wl_shm` buffer -> `copy` ->
//! await `flags`?/`ready`/`failed` -> read pixels -> [`Frame`].
//!
//! # Execution model
//!
//! Identical to the ICC backend: every operation runs on a short-lived worker
//! thread holding a ONE-SHOT Wayland connection ([`run`]), bridged to async
//! through the shared [`spawn_worker`] future. Closing the connection is the
//! universal cleanup, so a failed or cancelled capture never leaks frames or
//! buffers compositor-side. The deadline-bounded dispatch ([`wait`]) and the
//! `wl_shm` buffer ([`shm`]) are shared with the ICC backend, generic over the
//! backend error.
//!
//! # Orientation
//!
//! `wlr-screencopy` delivers buffers in the output's NATIVE (pre-transform)
//! orientation - the shared [`Frame`] contract - so there is no inverse remap
//! (see [`protocol`]); the only correction is the renderer's `y_invert` flag.
//!
//! # Cursor and permissions
//!
//! Cursor inclusion is the protocol's `overlay_cursor` capture flag (driven by
//! [`CaptureOpts::paint_cursor`]); there is no cursor position/image stream, so
//! [`cursor_events`](CaptureBackend::cursor_events) is `None` (a degradation,
//! never a failure). `Hyprland` gates screencopy behind the same permission
//! system as ICC, delivering a denial black frame instead of failing, so the
//! shared [`denial`] classifier maps it to [`PermissionResult::Denied`].
//!
//! [`run`]: crate::screencopy::run
//! [`protocol`]: crate::screencopy::protocol
//! [`wait`]: crate::icc::wait
//! [`shm`]: crate::icc::shm
//! [`denial`]: crate::denial
//! [`spawn_worker`]: crate::worker::spawn_worker

pub(crate) mod dispatch;
pub(crate) mod protocol;
mod run;

use async_trait::async_trait;
use flowshot_capture::{
    BackendKind, CaptureBackend, CaptureError, CaptureOpts, CursorStream, Frame, PermissionResult,
};
use flowshot_core::geometry::{LogicalRect, OutputInfo};

use crate::error::ScreencopyError;
use crate::worker::spawn_worker;

/// The `wlr-screencopy-unstable-v1` capture backend.
///
/// Stateless: each operation opens its own one-shot compositor connection on a
/// worker thread (see the module docs). Cheap to construct, `Send + Sync`, and
/// safe to drop at any point.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScreencopyBackend;

impl ScreencopyBackend {
    /// Creates the backend.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Captures the single output with this connector name.
    ///
    /// Mirrors [`IccBackend::capture_output_named`]: used by the QA harnesses
    /// (forced bad output name -> typed error) and callers targeting one output.
    ///
    /// # Errors
    ///
    /// [`CaptureError::Backend`] wrapping [`ScreencopyError::OutputNotFound`]
    /// (listing the available connectors) when the session does not advertise
    /// `connector`, plus every error of [`CaptureBackend::capture_outputs`] for
    /// the capture itself.
    ///
    /// [`IccBackend::capture_output_named`]: crate::IccBackend::capture_output_named
    pub async fn capture_output_named(
        &self,
        connector: &str,
        opts: CaptureOpts,
    ) -> Result<Frame, CaptureError> {
        let name = connector.to_owned();
        let captured = spawn_worker("flowshot-screencopy-named", move || {
            run::capture_run(opts, &run::Selection::Named(name))
        })
        .await?;
        let frame = captured
            .frames
            .into_iter()
            .next()
            .ok_or_else(|| CaptureError::Backend {
                backend: BackendKind::WlrScreencopy,
                source: ScreencopyError::Internal("a named capture returned no frame").into(),
            })?;
        Ok(frame)
    }
}

#[async_trait]
impl CaptureBackend for ScreencopyBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::WlrScreencopy
    }

    async fn outputs(&self) -> Result<Vec<OutputInfo>, CaptureError> {
        spawn_worker("flowshot-screencopy-outputs", run::outputs_run).await
    }

    async fn capture_outputs(&self, opts: CaptureOpts) -> Result<Vec<Frame>, CaptureError> {
        let captured = spawn_worker("flowshot-screencopy-capture", move || {
            run::capture_run(opts, &run::Selection::All)
        })
        .await?;
        Ok(captured.frames)
    }

    async fn capture_region(&self, region: LogicalRect) -> Result<Frame, CaptureError> {
        // Region captures never paint the cursor: the stitched composite is
        // content for editors and savers, matching the ICC backend and the
        // MockBackend contract.
        let captured = spawn_worker("flowshot-screencopy-region", || {
            run::capture_run(CaptureOpts::new(false), &run::Selection::All)
        })
        .await?;
        captured.stitch(BackendKind::WlrScreencopy, region)
    }

    fn cursor_events(&self) -> Option<CursorStream> {
        // wlr-screencopy has no cursor-observation protocol: the only cursor
        // control is the overlay_cursor capture flag (painted into the frame).
        // None is the contract's degradation, never a capture failure.
        None
    }

    async fn request_permission(&self) -> PermissionResult {
        match spawn_worker("flowshot-screencopy-permission", || {
            run::capture_run(CaptureOpts::new(false), &run::Selection::First)
        })
        .await
        {
            Ok(_captured) => PermissionResult::Granted,
            Err(CaptureError::Backend { source, .. })
                if matches!(
                    source.downcast_ref::<ScreencopyError>(),
                    Some(ScreencopyError::PermissionDenied)
                ) =>
            {
                PermissionResult::Denied
            }
            Err(error) => {
                // Never hang, never guess "granted": a failed probe reports the
                // safe side and logs the real cause.
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
        assert_eq!(ScreencopyBackend::new().kind(), BackendKind::WlrScreencopy);
    }

    #[test]
    fn cursor_events_degrades_to_none() {
        assert!(ScreencopyBackend::new().cursor_events().is_none());
    }

    #[test]
    fn screencopy_only_probe_negotiates_to_screencopy_first() {
        // A session advertising wlr-screencopy but NOT ICC (niri, pre-0.19
        // wlroots): negotiation must select WlrScreencopy as the first rung.
        let globals = vec![
            Global::new(1, "zwlr_screencopy_manager_v1", 3),
            Global::new(2, "zxdg_output_manager_v1", 3),
            Global::new(3, "wl_shm", 1),
        ];
        let probe: CapabilityProbe =
            ProtocolGlobals::from_globals(&globals).to_capability_probe(DesktopEnv::Niri);
        assert!(probe.supports(BackendKind::WlrScreencopy));
        assert!(!probe.supports(BackendKind::ExtImageCopyCapture));
        let ladder = negotiate(&probe, None).unwrap();
        assert_eq!(ladder.first(), Some(&BackendKind::WlrScreencopy));
    }

    #[test]
    fn forced_screencopy_wins_over_a_higher_rung() {
        // Hyprland advertises both ICC and screencopy; forcing screencopy (the
        // config override / this backend's whole reason to exist) selects it
        // even though ICC outranks it in the ladder.
        let globals = vec![
            Global::new(1, "ext_image_copy_capture_manager_v1", 1),
            Global::new(2, "ext_output_image_capture_source_manager_v1", 1),
            Global::new(3, "zwlr_screencopy_manager_v1", 3),
        ];
        let probe =
            ProtocolGlobals::from_globals(&globals).to_capability_probe(DesktopEnv::Hyprland);
        let forced = negotiate(&probe, Some(BackendKind::WlrScreencopy)).unwrap();
        assert_eq!(forced, vec![BackendKind::WlrScreencopy]);
    }
}
