//! The [`CaptureBackend`](flowshot_capture::CaptureBackend) implementation
//! of [`PortalScreenCastBackend`](super::super::PortalScreenCastBackend).

use ashpd::desktop::screencast::Screencast;
use async_trait::async_trait;
use flowshot_capture::{
    BackendKind, CaptureBackend, CaptureError, CaptureOpts, CursorStream, Frame, PermissionResult,
};
use flowshot_core::geometry::{LogicalRect, OutputInfo};

use super::super::error::PortalScreenCastError;
use super::super::run::{
    PORTAL_TIMEOUT, Selection, build_runtime, classify, collect_outputs, with_deadline,
};
use super::{close_session, screencast_run};
use crate::worker::spawn_worker;

#[async_trait]
impl CaptureBackend for super::super::PortalScreenCastBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::PortalScreenCast
    }

    async fn outputs(&self) -> Result<Vec<OutputInfo>, CaptureError> {
        spawn_worker("flowshot-portal-screencast-outputs", || {
            collect_outputs::<PortalScreenCastError>()
        })
        .await
    }

    async fn capture_outputs(&self, opts: CaptureOpts) -> Result<Vec<Frame>, CaptureError> {
        let captured = spawn_worker("flowshot-portal-screencast-capture", move || {
            screencast_run(opts, &Selection::All)
        })
        .await?;
        Ok(captured.frames)
    }

    async fn capture_region(&self, region: LogicalRect) -> Result<Frame, CaptureError> {
        // Region captures never paint the cursor: the stitched composite is
        // content for editors and savers, matching the other backends.
        let captured = spawn_worker("flowshot-portal-screencast-region", || {
            screencast_run(CaptureOpts::new(false), &Selection::All)
        })
        .await?;
        captured.stitch(BackendKind::PortalScreenCast, region)
    }

    fn cursor_events(&self) -> Option<CursorStream> {
        // The ScreenCast portal's only cursor channel is CursorMode::Metadata
        // (a PipeWire metadata stream v1 does not consume); cursor inclusion
        // is the Embedded capture flag. None is the contract's degradation.
        None
    }

    async fn request_permission(&self) -> PermissionResult {
        // Probe = CreateSession + close: proves the portal is alive and
        // accepting sessions WITHOUT popping the implementation's source
        // picker at a human. The real per-capture decision (the picker)
        // surfaces as Denied from capture_outputs.
        let probe = || {
            let runtime = build_runtime::<PortalScreenCastError>()?;
            runtime.block_on(with_deadline(PORTAL_TIMEOUT, session_probe()))
        };
        match spawn_worker("flowshot-portal-screencast-permission", probe).await {
            Ok(()) => PermissionResult::Granted,
            Err(CaptureError::Backend { source, .. })
                if source
                    .downcast_ref::<PortalScreenCastError>()
                    .is_some_and(PortalScreenCastError::is_denied) =>
            {
                PermissionResult::Denied
            }
            Err(error) => {
                tracing::warn!(%error, "portal permission probe failed; reporting denied");
                PermissionResult::Denied
            }
        }
    }
}

/// Creates and immediately closes a portal session (the permission probe).
async fn session_probe() -> Result<(), PortalScreenCastError> {
    let proxy: Screencast<'static> = classify(Screencast::new().await)?;
    let session = classify(proxy.create_session().await)?;
    close_session(session).await;
    Ok(())
}
