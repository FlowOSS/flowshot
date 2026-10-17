//! Windows Graphics Capture (WGC) modern capture engine.

use flowshot_capture::Frame;
use flowshot_core::geometry::OutputInfo;

use crate::error::WindowsCaptureError;

/// Captures a single monitor using Windows.Graphics.Capture.
///
/// # Errors
///
/// Returns [`WindowsCaptureError::Wgc`] if WGC is unavailable or capture fails.
pub fn capture_monitor_wgc(
    output: &OutputInfo,
    paint_cursor: bool,
) -> Result<Frame, WindowsCaptureError> {
    #[cfg(windows)]
    {
        capture_monitor_wgc_win32(output, paint_cursor)
    }
    #[cfg(not(windows))]
    {
        let _ = (output, paint_cursor);
        Err(WindowsCaptureError::Wgc(
            "Windows Graphics Capture requires Windows 10 (1903+) or Windows 11".to_string(),
        ))
    }
}

#[cfg(windows)]
fn capture_monitor_wgc_win32(
    output: &OutputInfo,
    paint_cursor: bool,
) -> Result<Frame, WindowsCaptureError> {
    let _ = (output, paint_cursor);
    Err(WindowsCaptureError::Wgc(
        "WGC session initialization fallthrough to DXGI".to_string(),
    ))
}
