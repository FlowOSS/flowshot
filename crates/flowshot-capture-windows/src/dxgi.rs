//! Direct3D 11 DXGI Desktop Duplication capture engine.

use flowshot_capture::Frame;
use flowshot_core::geometry::OutputInfo;

use crate::error::WindowsCaptureError;

/// Whether a captured DXGI frame buffer is unpopulated or contains only zeros.
#[must_use]
pub fn is_unpopulated_or_blank(accumulated_frames: u32, data: &[u8]) -> bool {
    accumulated_frames == 0 || data.iter().all(|&b| b == 0)
}

///
/// # Errors
///
/// Returns [`WindowsCaptureError::Dxgi`] if DXGI duplication is unsupported,
/// device creation fails, or frame acquisition fails.
pub fn capture_monitor_dxgi(
    output: &OutputInfo,
    paint_cursor: bool,
) -> Result<Frame, WindowsCaptureError> {
    #[cfg(windows)]
    {
        capture_monitor_dxgi_win32(output, paint_cursor)
    }
    #[cfg(not(windows))]
    {
        let _ = (output, paint_cursor);
        Err(WindowsCaptureError::Dxgi(
            "DXGI requires a Windows environment".to_string(),
        ))
    }
}

#[cfg(windows)]
#[expect(
    clippy::too_many_lines,
    clippy::borrow_as_ptr,
    clippy::collapsible_if,
    reason = "Win32 DXGI FFI structures require raw pointers and long initialization"
)]
fn capture_monitor_dxgi_win32(
    output: &OutputInfo,
    paint_cursor: bool,
) -> Result<Frame, WindowsCaptureError> {
    use bytes::BytesMut;
    use flowshot_capture::{FrameBuffer, FrameFormat, OutputRef};
    use std::mem;
    use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0};
    use windows::Win32::Graphics::Direct3D11::{
        D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
        D3D11_MAPPED_SUBRESOURCE, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING, D3D11CreateDevice,
        ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
    };
    use windows::Win32::Graphics::Dxgi::{
        DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO, IDXGIAdapter, IDXGIDevice, IDXGIOutput1,
        IDXGIOutputDuplication, IDXGIResource,
    };
    use windows::core::Interface;

    let _ = paint_cursor;
    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;
    let feature_levels = [D3D_FEATURE_LEVEL_11_0];

    let hr = unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            windows::Win32::Foundation::HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&feature_levels),
            7,
            Some(&mut device),
            None,
            Some(&mut context),
        )
    };
    if hr.is_err() || device.is_none() || context.is_none() {
        return Err(WindowsCaptureError::Dxgi(format!(
            "D3D11CreateDevice failed: {hr:?}"
        )));
    }
    let device =
        device.ok_or_else(|| WindowsCaptureError::Dxgi("missing D3D11 device".to_string()))?;
    let context =
        context.ok_or_else(|| WindowsCaptureError::Dxgi("missing D3D11 context".to_string()))?;

    let dxgi_device: IDXGIDevice = device
        .cast()
        .map_err(|e| WindowsCaptureError::Dxgi(e.to_string()))?;
    let adapter: IDXGIAdapter = unsafe { dxgi_device.GetAdapter() }
        .map_err(|e| WindowsCaptureError::Dxgi(e.to_string()))?;

    let mut output_idx = 0u32;
    let mut target_output: Option<IDXGIOutput1> = None;

    while let Ok(dxgi_output) = unsafe { adapter.EnumOutputs(output_idx) } {
        if let Ok(output1) = dxgi_output.cast::<IDXGIOutput1>() {
            if let Ok(desc) = unsafe { dxgi_output.GetDesc() } {
                let name = String::from_utf16_lossy(&desc.DeviceName)
                    .trim_matches('\0')
                    .to_string();
                if name == output.connector || output_idx == 0 {
                    target_output = Some(output1);
                    if name == output.connector {
                        break;
                    }
                }
            }
        }
        output_idx += 1;
    }

    let Some(target_output) = target_output else {
        return Err(WindowsCaptureError::Dxgi(
            "no matching DXGI output found".to_string(),
        ));
    };

    let dupl: IDXGIOutputDuplication = unsafe { target_output.DuplicateOutput(&device) }
        .map_err(|e| WindowsCaptureError::Dxgi(format!("DuplicateOutput failed: {e}")))?;

    let mut frame_info: DXGI_OUTDUPL_FRAME_INFO = unsafe { mem::zeroed() };
    let mut resource: Option<IDXGIResource> = None;

    let acq = unsafe { dupl.AcquireNextFrame(200, &mut frame_info, &mut resource) };
    if let Err(e) = &acq {
        if e.code() == DXGI_ERROR_WAIT_TIMEOUT {
            return Err(WindowsCaptureError::Timeout);
        }
        return Err(WindowsCaptureError::Dxgi(format!(
            "AcquireNextFrame failed: {e}"
        )));
    }
    if resource.is_none() {
        return Err(WindowsCaptureError::Dxgi(
            "AcquireNextFrame delivered empty resource".to_string(),
        ));
    }
    let resource =
        resource.ok_or_else(|| WindowsCaptureError::Dxgi("missing DXGI resource".to_string()))?;

    let desktop_texture: ID3D11Texture2D = resource
        .cast()
        .map_err(|e| WindowsCaptureError::Dxgi(e.to_string()))?;

    let mut desc: D3D11_TEXTURE2D_DESC = unsafe { mem::zeroed() };
    unsafe { desktop_texture.GetDesc(&mut desc) };

    desc.Usage = D3D11_USAGE_STAGING;
    desc.BindFlags = 0;
    desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
    desc.MiscFlags = 0;

    let mut staging: Option<ID3D11Texture2D> = None;
    unsafe {
        device
            .CreateTexture2D(&desc, None, Some(&mut staging))
            .map_err(|e| WindowsCaptureError::Dxgi(e.to_string()))?;
    }
    let staging =
        staging.ok_or_else(|| WindowsCaptureError::Dxgi("missing staging texture".to_string()))?;

    unsafe {
        context.CopyResource(&staging, &desktop_texture);
    }
    let mut mapped: D3D11_MAPPED_SUBRESOURCE = unsafe { mem::zeroed() };
    unsafe {
        context
            .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
            .map_err(|e| WindowsCaptureError::Dxgi(e.to_string()))?;
    }

    let width = desc.Width;
    let height = desc.Height;
    let row_pitch = mapped.RowPitch as usize;
    let src_slice = unsafe {
        std::slice::from_raw_parts(mapped.pData as *const u8, row_pitch * height as usize)
    };

    let mut data = BytesMut::zeroed((width * height * 4) as usize);
    for y in 0..height as usize {
        let src_start = y * row_pitch;
        let dst_start = y * (width as usize * 4);
        data[dst_start..dst_start + (width as usize * 4)]
            .copy_from_slice(&src_slice[src_start..src_start + (width as usize * 4)]);
    }

    unsafe {
        context.Unmap(&staging, 0);
        let _ = dupl.ReleaseFrame();
    }

    if is_unpopulated_or_blank(frame_info.AccumulatedFrames, &data) {
        return Err(WindowsCaptureError::Dxgi(
            "DXGI delivered unpopulated or all-zero frame; falling through to GDI BitBlt"
                .to_string(),
        ));
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpopulated_or_blank_identifies_fallthrough_conditions() {
        assert!(is_unpopulated_or_blank(0, &[1, 2, 3]));
        assert!(is_unpopulated_or_blank(1, &[0, 0, 0, 0]));
        assert!(!is_unpopulated_or_blank(1, &[0, 0, 1, 0]));
    }
}
