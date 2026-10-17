//! Display monitor enumeration and DPI awareness for Windows.

use flowshot_core::geometry::{
    Logical, LogicalRect, OutputInfo, PhysicalPx, PhysicalSize, Transform,
};

use crate::error::WindowsCaptureError;

/// Enumerates all active display monitors with their physical sizes and scales.
///
/// # Errors
///
/// Returns [`WindowsCaptureError::NoMonitors`] if no active displays are found.
pub fn enumerate_monitors() -> Result<Vec<OutputInfo>, WindowsCaptureError> {
    #[cfg(windows)]
    {
        enumerate_monitors_win32()
    }
    #[cfg(not(windows))]
    {
        Ok(vec![
            OutputInfo::new(
                r"\\.\DISPLAY1",
                "Primary Display",
                LogicalRect::new(Logical(0.0), Logical(0.0), Logical(1920.0), Logical(1080.0)),
                PhysicalSize::new(PhysicalPx(1920), PhysicalPx(1080)),
                1.0,
                Transform::Normal,
            )
            .map_err(|e| WindowsCaptureError::Platform(e.to_string()))?,
        ])
    }
}

#[cfg(windows)]
#[expect(
    clippy::borrow_as_ptr,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_lossless,
    clippy::ptr_as_ptr,
    reason = "Win32 monitor enumeration callback requires FFI casts and raw pointer passing"
)]
fn enumerate_monitors_win32() -> Result<Vec<OutputInfo>, WindowsCaptureError> {
    use std::mem;
    use windows::Win32::Foundation::{LPARAM, RECT};
    use windows::Win32::Graphics::Gdi::{
        EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW,
    };
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    use windows::core::BOOL;

    struct EnumContext {
        monitors: Vec<OutputInfo>,
    }

    unsafe extern "system" fn monitor_enum_proc(
        hmonitor: HMONITOR,
        _hdc: HDC,
        _rect: *mut RECT,
        lparam: LPARAM,
    ) -> BOOL {
        let context = unsafe { &mut *(lparam.0 as *mut EnumContext) };
        let mut info: MONITORINFOEXW = unsafe { mem::zeroed() };
        info.monitorInfo.cbSize = mem::size_of::<MONITORINFOEXW>() as u32;

        let success = unsafe { GetMonitorInfoW(hmonitor, &mut info as *mut _ as *mut _) };
        if !success.as_bool() {
            return BOOL(1);
        }

        let rc = info.monitorInfo.rcMonitor;
        let left = rc.left as f64;
        let top = rc.top as f64;
        let width = (rc.right - rc.left).max(0) as f64;
        let height = (rc.bottom - rc.top).max(0) as f64;

        let mut dpi_x = 96u32;
        let mut dpi_y = 96u32;
        let _ = unsafe { GetDpiForMonitor(hmonitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
        let scale = (dpi_x as f64) / 96.0;

        let device_name = String::from_utf16_lossy(&info.szDevice)
            .trim_matches('\0')
            .to_string();

        let physical_w = width as u32;
        let physical_h = height as u32;
        let logical_w = width / scale;
        let logical_h = height / scale;
        let logical_x = left / scale;
        let logical_y = top / scale;

        if let Ok(output) = OutputInfo::new(
            &device_name,
            &device_name,
            LogicalRect::new(
                Logical(logical_x),
                Logical(logical_y),
                Logical(logical_w),
                Logical(logical_h),
            ),
            PhysicalSize::new(PhysicalPx(physical_w as i32), PhysicalPx(physical_h as i32)),
            scale,
            Transform::Normal,
        ) {
            context.monitors.push(output);
        }

        BOOL(1)
    }

    let mut context = EnumContext {
        monitors: Vec::new(),
    };
    let success = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(monitor_enum_proc),
            LPARAM(&mut context as *mut _ as isize),
        )
    };

    if !success.as_bool() || context.monitors.is_empty() {
        return Err(WindowsCaptureError::NoMonitors);
    }

    Ok(context.monitors)
}
