//! Universal Win32 GDI screen capture fallback.

use bytes::BytesMut;
use flowshot_capture::{Frame, FrameBuffer, FrameFormat, OutputRef};
use flowshot_core::geometry::OutputInfo;

use crate::error::WindowsCaptureError;

/// Captures a single monitor using Win32 GDI `BitBlt` and DIB section.
///
/// # Errors
///
/// Returns [`WindowsCaptureError`] if acquiring the DC, allocating the DIB, or `BitBlt` fails.
pub fn capture_monitor_gdi(
    output: &OutputInfo,
    paint_cursor: bool,
) -> Result<Frame, WindowsCaptureError> {
    #[cfg(windows)]
    {
        capture_monitor_gdi_win32(output, paint_cursor)
    }
    #[cfg(not(windows))]
    {
        let _ = paint_cursor;
        let width = output.physical_size.width.0.max(0).cast_unsigned();
        let height = output.physical_size.height.0.max(0).cast_unsigned();
        let data = BytesMut::zeroed((width * height * 4) as usize);
        let buffer = FrameBuffer {
            data,
            width,
            height,
            stride: width * 4,
            format: FrameFormat::Xrgb8888,
        };
        Ok(Frame {
            buffer,
            output: OutputRef::Connector(output.connector.clone()),
            scale: output.scale,
            transform: output.transform,
        })
    }
}

#[cfg(windows)]
#[expect(
    clippy::too_many_lines,
    clippy::borrow_as_ptr,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::unnecessary_cast,
    reason = "Win32 GDI FFI structures require casts and raw pointer passing"
)]
fn capture_monitor_gdi_win32(
    output: &OutputInfo,
    paint_cursor: bool,
) -> Result<Frame, WindowsCaptureError> {
    use std::ffi::c_void;
    use std::mem;
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleDC,
        CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, ReleaseDC, SRCCOPY,
        SelectObject,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CURSOR_SHOWING, CURSORINFO, DI_NORMAL, DrawIconEx, GetCursorInfo, GetIconInfo,
    };

    let width = output.physical_size.width.0 as i32;
    let height = output.physical_size.height.0 as i32;
    let origin_x = (output.logical_rect.origin().x.0 * output.scale).round() as i32;
    let origin_y = (output.logical_rect.origin().y.0 * output.scale).round() as i32;

    let hdc_screen = unsafe { GetDC(None) };
    if hdc_screen.is_invalid() {
        return Err(WindowsCaptureError::DeviceContext(
            "GetDC(HWND::default()) failed".to_string(),
        ));
    }

    let hdc_mem = unsafe { CreateCompatibleDC(Some(hdc_screen)) };
    if hdc_mem.is_invalid() {
        unsafe { ReleaseDC(None, hdc_screen) };
        return Err(WindowsCaptureError::DeviceContext(
            "CreateCompatibleDC failed".to_string(),
        ));
    }

    let mut bmi: BITMAPINFO = unsafe { mem::zeroed() };
    bmi.bmiHeader.biSize = mem::size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = width;
    bmi.bmiHeader.biHeight = -height;
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = BI_RGB.0;

    let mut bits: *mut c_void = std::ptr::null_mut();
    let hbitmap =
        unsafe { CreateDIBSection(Some(hdc_screen), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) };

    if hbitmap.is_err() || bits.is_null() {
        unsafe {
            let _ = DeleteDC(hdc_mem);
            ReleaseDC(None, hdc_screen);
        }
        return Err(WindowsCaptureError::CreateDib(
            "CreateDIBSection failed".to_string(),
        ));
    }
    let hbitmap = hbitmap.map_err(|e| WindowsCaptureError::CreateDib(e.to_string()))?;

    let old_bmp = unsafe { SelectObject(hdc_mem, hbitmap.into()) };

    let blt_res = unsafe {
        BitBlt(
            hdc_mem,
            0,
            0,
            width,
            height,
            Some(hdc_screen),
            origin_x,
            origin_y,
            SRCCOPY | CAPTUREBLT,
        )
    };

    if blt_res.is_err() {
        unsafe {
            SelectObject(hdc_mem, old_bmp);
            let _ = DeleteObject(hbitmap.into());
            let _ = DeleteDC(hdc_mem);
            ReleaseDC(None, hdc_screen);
        }
        return Err(WindowsCaptureError::BitBlt(
            "BitBlt raster transfer failed".to_string(),
        ));
    }

    if paint_cursor {
        let mut ci: CURSORINFO = unsafe { mem::zeroed() };
        ci.cbSize = mem::size_of::<CURSORINFO>() as u32;
        if unsafe { GetCursorInfo(&mut ci) }.is_ok() && (ci.flags.0 & CURSOR_SHOWING.0 != 0) {
            let mut ii = unsafe { mem::zeroed() };
            if unsafe {
                GetIconInfo(
                    windows::Win32::UI::WindowsAndMessaging::HICON(ci.hCursor.0),
                    &mut ii,
                )
            }
            .is_ok()
            {
                let cursor_x = ci.ptScreenPos.x - origin_x - (ii.xHotspot as i32);
                let cursor_y = ci.ptScreenPos.y - origin_y - (ii.yHotspot as i32);
                let _ = unsafe {
                    DrawIconEx(
                        hdc_mem,
                        cursor_x,
                        cursor_y,
                        windows::Win32::UI::WindowsAndMessaging::HICON(ci.hCursor.0),
                        0,
                        0,
                        0,
                        None,
                        DI_NORMAL,
                    )
                };
                if !ii.hbmMask.is_invalid() {
                    let _ = unsafe { DeleteObject(ii.hbmMask.into()) };
                }
                if !ii.hbmColor.is_invalid() {
                    let _ = unsafe { DeleteObject(ii.hbmColor.into()) };
                }
            }
        }
    }

    let byte_count = (width * height * 4) as usize;
    let data = unsafe { std::slice::from_raw_parts(bits as *const u8, byte_count) };
    let buffer_data = BytesMut::from(data);

    unsafe {
        SelectObject(hdc_mem, old_bmp);
        let _ = DeleteObject(hbitmap.into());
        let _ = DeleteDC(hdc_mem);
        ReleaseDC(None, hdc_screen);
    }

    let u_width = width as u32;
    let u_height = height as u32;
    let stride = u_width * 4;
    let buffer = FrameBuffer {
        data: buffer_data,
        width: u_width,
        height: u_height,
        stride,
        format: FrameFormat::Xrgb8888,
    };

    Ok(Frame {
        buffer,
        output: OutputRef::Connector(output.connector.clone()),
        scale: output.scale,
        transform: output.transform,
    })
}
