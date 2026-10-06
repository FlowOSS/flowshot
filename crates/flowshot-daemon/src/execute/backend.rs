//! The capture-ladder runner: probe the live session,
//! negotiate the backend order, and construct the first backend that
//! actually serves outputs - the negotiation ladder made executable.
//!
//! Session routing (plan decision #7): `WAYLAND_DISPLAY` set-and-nonempty
//! selects the Wayland leg; else `DISPLAY` set-and-nonempty selects the X11
//! leg; with neither variable the Wayland leg's spawn produces the existing
//! typed connect error. The routing rule is the clipboard crate's
//! [`detect_session`] - one implementation of the decision, shared by the
//! capture leg and the clipboard backend pick.
//!
//! [`detect_session`]: flowshot_actions::clipboard::detect_session

use flowshot_actions::clipboard::{SessionKind, detect_session};
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
    /// The outputs as probed (registry order = the indexing
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
/// capture - e.g. the rotated-headless `BufferSizeMismatch` class -
/// and the ladder's promise is the next rung, not a hard error).
///
/// # Errors
///
/// Same as [`open_session`]; `CaptureError::NoBackendAvailable` names the
/// exhausted rungs.
pub async fn open_session_excluding(
    exclude: &[BackendKind],
) -> Result<CaptureSession, ExecuteError> {
    match detect_session() {
        Ok(SessionKind::X11) => {
            tracing::debug!(session = "x11", "capture session routing");
            return open_x11_session(exclude).await;
        }
        // A Wayland session - or no session variable at all, where the leg
        // below produces the existing typed connect error (path unchanged).
        Ok(SessionKind::Wayland) | Err(_) => {
            tracing::debug!(session = "wayland", "capture session routing");
        }
    }
    let thread = flowshot_capture_wayland::CaptureThread::spawn()?;
    let probe = thread.probe()?;
    let outputs = thread.outputs()?;
    thread.shutdown();
    let kinds = negotiate(&probe, None)?;
    tracing::info!(
        session = "wayland",
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
        let backend = match construct(kind) {
            Ok(backend) => backend,
            Err(error) => {
                tracing::warn!(backend = ?kind, %error, "ladder rung failed at construction; falling through");
                tried.push(kind);
                last = Some(error);
                continue;
            }
        };
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

/// The X11 leg (plan decision #7): [`probe_x11`] feeds the same
/// negotiation ladder (yielding the single X11 rung); a failed probe is
/// the existing typed no-backend error. Mirrors the Wayland leg's ladder
/// walk deliberately - sibling platform legs stay parallel implementations
/// (the `stitch.rs` duplication precedent), the Wayland leg is not
/// refactored.
///
/// [`probe_x11`]: flowshot_capture_x11::probe_x11
async fn open_x11_session(exclude: &[BackendKind]) -> Result<CaptureSession, ExecuteError> {
    let Some(probe) = flowshot_capture_x11::probe_x11() else {
        // DISPLAY is set but the session is not capturable (an unreachable
        // server, RANDR < 1.2). `probe_x11` logs the reason at debug level;
        // name the leg at warn so the default log level stays diagnosable.
        tracing::warn!(
            "the X11 session probe found no capturable server \
             (RUST_LOG=flowshot_capture_x11=debug names the reason)"
        );
        return Err(ExecuteError::Capture(CaptureError::NoBackendAvailable {
            missing: vec![BackendKind::X11],
        }));
    };
    let kinds = negotiate(&probe, None)?;
    tracing::info!(
        session = "x11",
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
        let backend = match construct(kind) {
            Ok(backend) => backend,
            Err(error) => {
                tracing::warn!(backend = ?kind, %error, "ladder rung failed at construction; falling through");
                tried.push(kind);
                last = Some(error);
                continue;
            }
        };
        match backend.outputs().await {
            Ok(live) => {
                tracing::info!(backend = ?kind, outputs = live.len(), "capture backend ready");
                return Ok(CaptureSession {
                    backend,
                    kind,
                    outputs: live,
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

fn construct(kind: BackendKind) -> Result<Box<dyn CaptureBackend>, CaptureError> {
    Ok(match kind {
        BackendKind::ExtImageCopyCapture => Box::new(IccBackend::new()),
        BackendKind::WlrScreencopy => Box::new(ScreencopyBackend::new()),
        BackendKind::KwinScreenShot2 => Box::new(KwinScreenShot2Backend::new()),
        BackendKind::PortalScreenCast => Box::new(PortalScreenCastBackend::new()),
        BackendKind::PortalScreenshot => Box::new(PortalScreenshotBackend::new()),
        // The X11 rung connects eagerly (probing the capture caps); the
        // connect failure rides the crate's X11Error -> CaptureError
        // conversion, and the warn keeps the DISPLAY hint (which the
        // CaptureError display does not carry) in the default-level log.
        BackendKind::X11 => Box::new(
            flowshot_capture_x11::X11Backend::connect()
                .inspect_err(|error| tracing::warn!(%error, "X11 backend connect failed"))?,
        ),
        // Roadmap kinds never come out of `negotiate` (v1 gate); treating
        // them as a backend absence keeps the match exhaustive without a
        // panic path.
        BackendKind::Windows | BackendKind::MacOs => {
            tracing::error!(?kind, "roadmap backend kind reached construction");
            Box::new(IccBackend::new())
        }
    })
}

/// Resolves the live cursor through the session's strategy: the layered
/// Wayland ladder (ICC cursor session -> Hyprland IPC -> first-motion
/// deferral) or the X11 one-shot `XQueryPointer` read.
pub async fn resolve_cursor() -> Option<LogicalPoint> {
    match detect_session() {
        Ok(SessionKind::X11) => resolve_cursor_x11().await,
        Ok(SessionKind::Wayland) | Err(_) => resolve_cursor_wayland().await,
    }
}

async fn resolve_cursor_wayland() -> Option<LogicalPoint> {
    let source = flowshot_capture_wayland::resolve_cursor_pos(Some(IccBackend::new())).await;
    let position = source.position();
    tracing::info!(
        layer = source.layer_name(),
        resolved = position.is_some(),
        "cursor position ladder"
    );
    position.map(|(x, y)| LogicalPoint::from_raw(f64::from(x), f64::from(y)))
}

/// X11 has no cursor stream in Phase A: one-shot `XQueryPointer`. Every
/// failure degrades to `None` - a missing preselect never fails a capture.
async fn resolve_cursor_x11() -> Option<LogicalPoint> {
    let backend = match flowshot_capture_x11::X11Backend::connect() {
        Ok(backend) => backend,
        Err(error) => {
            tracing::warn!(%error, "X11 cursor read unavailable; preselect degrades");
            return None;
        }
    };
    match backend.cursor_pos().await {
        Ok(position) => {
            tracing::info!(
                layer = "x11-query-pointer",
                resolved = position.is_some(),
                "cursor position ladder"
            );
            position
        }
        Err(error) => {
            tracing::warn!(%error, "X11 cursor position read failed; preselect degrades");
            None
        }
    }
}
