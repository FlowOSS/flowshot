//! Windows screen capture platform implementation for `FlowShot`.
//!
//! Provides a resilient, 3-tier capture negotiation ladder:
//! 1. Windows Graphics Capture (WGC) for Windows 11 and modern Windows 10
//! 2. DXGI Desktop Duplication for Windows 8+ and Windows Server 2012+
//! 3. Win32 GDI `BitBlt` universal bedrock fallback for Windows 7, RDP, and VMs

pub mod dxgi;
pub mod error;
pub mod gdi;
pub mod monitor;
pub mod wgc;

use async_trait::async_trait;
use flowshot_capture::stitch::CapturedOutputs;
use flowshot_capture::{
    BackendKind, CapabilityProbe, CaptureBackend, CaptureError, CaptureOpts, CursorStream, Frame,
    PermissionResult,
};
use flowshot_core::geometry::{LogicalRect, OutputInfo};

pub use error::WindowsCaptureError;

/// Windows capture backend running the 3-tier negotiation ladder.
#[derive(Debug, Default, Clone)]
pub struct WindowsBackend;

impl WindowsBackend {
    /// Creates a new Windows capture backend.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl CaptureBackend for WindowsBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Windows
    }

    async fn outputs(&self) -> Result<Vec<OutputInfo>, CaptureError> {
        monitor::enumerate_monitors().map_err(Into::into)
    }

    async fn capture_outputs(&self, opts: CaptureOpts) -> Result<Vec<Frame>, CaptureError> {
        let monitors = self.outputs().await?;
        let mut frames = Vec::with_capacity(monitors.len());

        for output in &monitors {
            let frame = match wgc::capture_monitor_wgc(output, opts.paint_cursor) {
                Ok(frame) => frame,
                Err(wgc_err) => {
                    tracing::debug!(%wgc_err, "WGC capture unavailable; falling through to DXGI");
                    match dxgi::capture_monitor_dxgi(output, opts.paint_cursor) {
                        Ok(frame) => frame,
                        Err(dxgi_err) => {
                            tracing::debug!(
                                %dxgi_err,
                                "DXGI capture unavailable; falling through to GDI BitBlt"
                            );
                            gdi::capture_monitor_gdi(output, opts.paint_cursor)?
                        }
                    }
                }
            };
            frames.push(frame);
        }

        Ok(frames)
    }

    async fn capture_region(&self, rect: LogicalRect) -> Result<Frame, CaptureError> {
        let outputs = self.outputs().await?;
        let frames = self.capture_outputs(CaptureOpts::default()).await?;
        let captured = CapturedOutputs { outputs, frames };
        captured.stitch(BackendKind::Windows, rect)
    }

    fn cursor_events(&self) -> Option<CursorStream> {
        None
    }

    async fn request_permission(&self) -> PermissionResult {
        PermissionResult::NotRequired
    }
}

/// Probes whether Windows capture is supported on the active environment.
#[must_use]
pub fn probe_windows() -> Option<CapabilityProbe> {
    #[cfg(windows)]
    {
        Some(CapabilityProbe::new(
            flowshot_capture::DesktopEnv::Other,
            [BackendKind::Windows],
        ))
    }
    #[cfg(not(windows))]
    {
        None
    }
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "unit test assertions")]
mod tests {
    use super::*;
    use flowshot_core::geometry::Logical;

    #[test]
    fn windows_backend_declares_windows_kind() {
        let backend = WindowsBackend::new();
        assert_eq!(backend.kind(), BackendKind::Windows);
    }

    #[tokio::test]
    async fn windows_backend_enumerates_monitors_and_captures_frames() {
        let backend = WindowsBackend::new();
        let outputs = backend.outputs().await.unwrap();
        assert!(!outputs.is_empty(), "must discover at least one monitor");

        let frames = backend
            .capture_outputs(CaptureOpts::new(true))
            .await
            .unwrap();
        assert_eq!(
            frames.len(),
            outputs.len(),
            "must produce one frame per monitor"
        );
        assert_eq!(
            frames[0].buffer.format,
            flowshot_capture::FrameFormat::Xrgb8888
        );
    }

    #[tokio::test]
    async fn windows_backend_captures_stitched_region() {
        let backend = WindowsBackend::new();
        let rect = LogicalRect::new(Logical(10.0), Logical(10.0), Logical(100.0), Logical(100.0));
        let frame = backend.capture_region(rect).await.unwrap();
        assert_eq!(frame.buffer.width, 100);
        assert_eq!(frame.buffer.height, 100);
        assert_eq!(frame.output, flowshot_capture::OutputRef::Composite);
    }
}
