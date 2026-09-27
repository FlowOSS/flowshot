//! The capture-ladder runner (plan todo 5/38): probe the live session,
//! negotiate the backend order, and construct the first backend that
//! actually serves outputs - the negotiation ladder made executable.

use flowshot_capture::{BackendKind, CaptureBackend, CaptureError, negotiate};
use flowshot_capture_wayland::{
    IccBackend, KwinScreenShot2Backend, PortalScreenCastBackend, PortalScreenshotBackend,
    ScreencopyBackend,
};
use flowshot_core::geometry::{LogicalPoint, OutputInfo};

use super::ExecuteError;

/// A negotiated backend paired with the layout it probed.
pub struct CaptureSession {
    /// The backend that won the ladder (its `outputs()` succeeded).
    pub backend: Box<dyn CaptureBackend>,
    /// Which ladder rung won (log/evidence token).
    pub kind: BackendKind,
    /// The outputs as probed (registry order = the todo-6 indexing
    /// authority for `capture screen <n>`).
    pub outputs: Vec<OutputInfo>,
}

impl std::fmt::Debug for CaptureSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CaptureSession")
            .field("kind", &self.kind)
            .field("outputs", &self.outputs.len())
            .finish_non_exhaustive()
    }
}

/// Probes the session, negotiates the ladder, and returns the first
/// backend that serves outputs.
///
/// # Errors
///
/// [`ExecuteError::Connect`]/[`ExecuteError::Probe`] for session failures,
/// [`ExecuteError::Capture`] when no ladder rung serves outputs.
pub async fn open_session() -> Result<CaptureSession, ExecuteError> {
    open_session_excluding(&[]).await
}

/// [`open_session`] skipping rungs that already failed at CAPTURE time
/// (the runtime fallthrough: a backend can probe green and still fail a
/// capture - e.g. the todo-7 rotated-headless `BufferSizeMismatch` class -
/// and the ladder's promise is the next rung, not a hard error).
///
/// # Errors
///
/// Same as [`open_session`]; `CaptureError::NoBackendAvailable` names the
/// exhausted rungs.
pub async fn open_session_excluding(
    exclude: &[BackendKind],
) -> Result<CaptureSession, ExecuteError> {
    let thread = flowshot_capture_wayland::CaptureThread::spawn()?;
    let probe = thread.probe()?;
    let outputs = thread.outputs()?;
    thread.shutdown();
    let kinds = negotiate(&probe, None)?;
    tracing::info!(
        ladder = ?kinds,
        desktop = ?probe.desktop,
        excluded = ?exclude,
        "capture ladder negotiated"
    );
    let mut tried: Vec<BackendKind> = Vec::new();
    let mut last: Option<CaptureError> = None;
    for kind in kinds {
        if exclude.contains(&kind) {
            tried.push(kind);
            continue;
        }
        let backend = construct(kind);
        match backend.outputs().await {
            Ok(live) => {
                tracing::info!(backend = ?kind, outputs = live.len(), "capture backend ready");
                return Ok(CaptureSession {
                    backend,
                    kind,
                    outputs: if live.is_empty() { outputs } else { live },
                });
            }
            Err(error) => {
                tracing::warn!(backend = ?kind, %error, "ladder rung failed; falling through");
                tried.push(kind);
                last = Some(error);
            }
        }
    }
    if let Some(error) = last {
        tracing::error!(?tried, "every ladder rung failed");
        return Err(ExecuteError::Capture(error));
    }
    Err(ExecuteError::Capture(CaptureError::NoBackendAvailable {
        missing: tried,
    }))
}

fn construct(kind: BackendKind) -> Box<dyn CaptureBackend> {
    match kind {
        BackendKind::ExtImageCopyCapture => Box::new(IccBackend::new()),
        BackendKind::WlrScreencopy => Box::new(ScreencopyBackend::new()),
        BackendKind::KwinScreenShot2 => Box::new(KwinScreenShot2Backend::new()),
        BackendKind::PortalScreenCast => Box::new(PortalScreenCastBackend::new()),
        BackendKind::PortalScreenshot => Box::new(PortalScreenshotBackend::new()),
        // Roadmap kinds never come out of `negotiate` (v1 gate); treating
        // them as a backend absence keeps the match exhaustive without a
        // panic path (Amendment #4).
        BackendKind::X11 | BackendKind::Windows | BackendKind::MacOs => {
            tracing::error!(?kind, "roadmap backend kind reached construction");
            Box::new(IccBackend::new())
        }
    }
}

/// Resolves the live cursor through the todo-12 layered strategy (ICC
/// cursor session -> Hyprland IPC -> first-motion deferral).
pub async fn resolve_cursor() -> Option<LogicalPoint> {
    let source = flowshot_capture_wayland::resolve_cursor_pos(Some(IccBackend::new())).await;
    let position = source.position();
    tracing::info!(
        layer = source.layer_name(),
        resolved = position.is_some(),
        "cursor position ladder"
    );
    position.map(|(x, y)| LogicalPoint::from_raw(f64::from(x), f64::from(y)))
}
