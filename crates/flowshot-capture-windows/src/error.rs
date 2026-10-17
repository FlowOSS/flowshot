//! Error types for Windows screen capture.

use thiserror::Error;

/// Typed error for Windows capture operations.
#[derive(Debug, Error)]
pub enum WindowsCaptureError {
    /// No monitors were discovered or monitor enumeration failed.
    #[error("no display monitors discovered on the system")]
    NoMonitors,

    /// Win32 device context creation or acquisition failed.
    #[error("failed to acquire device context: {0}")]
    DeviceContext(String),

    /// Win32 DIB section creation failed.
    #[error("failed to create 32-bit DIB section: {0}")]
    CreateDib(String),

    /// Win32 `BitBlt` raster transfer failed.
    #[error("BitBlt screen capture failed: {0}")]
    BitBlt(String),

    /// DXGI device creation or desktop duplication failed.
    #[error("DXGI desktop duplication failed: {0}")]
    Dxgi(String),

    /// Windows Graphics Capture failed.
    #[error("Windows Graphics Capture failed: {0}")]
    Wgc(String),

    /// The operation timed out waiting for a frame.
    #[error("timed out waiting for capture frame")]
    Timeout,

    /// Screen capture permission was denied by the user or system policy.
    #[error("screen capture permission was denied")]
    PermissionDenied,

    /// General platform error.
    #[error("platform error: {0}")]
    Platform(String),
}

impl From<WindowsCaptureError> for flowshot_capture::CaptureError {
    fn from(err: WindowsCaptureError) -> Self {
        Self::Backend {
            backend: flowshot_capture::BackendKind::Windows,
            source: Box::new(err),
        }
    }
}
