//! Windows clipboard backend: Win32 clipboard with `CF_DIBV5`, `CF_UNICODETEXT`, and `CF_HDROP`.

use super::ClipboardBackend;
use super::offer::ClipboardOffer;
#[cfg(windows)]
use super::offer::{MIME_JPEG, MIME_PNG, MIME_TEXT_PLAIN};
use crate::error::ClipboardError;

/// Production Windows clipboard backend.
#[derive(Debug, Clone, Copy, Default)]
pub struct WindowsClipboard;

impl WindowsClipboard {
    /// Creates a new Windows clipboard backend.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl ClipboardBackend for WindowsClipboard {
    fn serve(&self, offer: ClipboardOffer) -> Result<(), ClipboardError> {
        #[cfg(windows)]
        {
            serve_windows(&offer)
        }
        #[cfg(not(windows))]
        {
            let _ = offer;
            Ok(())
        }
    }
}

/// Converts an encoded PNG/JPEG image buffer into 32-bit `CF_DIBV5` format bytes.
///
/// Returns the full memory buffer (`BITMAPV5HEADER` + bottom-up BGRA pixels).
#[cfg_attr(not(windows), allow(dead_code))]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    reason = "DIB header format"
)]
pub fn create_dibv5_buffer(image_bytes: &[u8]) -> Option<Vec<u8>> {
    let img = image::load_from_memory(image_bytes).ok()?.to_rgba8();
    let width = img.width();
    let height = img.height();

    let header_size = 124usize;
    let image_data_size = (width as usize) * (height as usize) * 4;
    let total_size = header_size + image_data_size;

    let mut buf = vec![0u8; total_size];

    buf[0..4].copy_from_slice(&(header_size as u32).to_le_bytes());
    buf[4..8].copy_from_slice(&(width as i32).to_le_bytes());
    buf[8..12].copy_from_slice(&(height as i32).to_le_bytes());
    buf[12..14].copy_from_slice(&1u16.to_le_bytes());
    buf[14..16].copy_from_slice(&32u16.to_le_bytes());
    buf[16..20].copy_from_slice(&3u32.to_le_bytes());
    buf[20..24].copy_from_slice(&(image_data_size as u32).to_le_bytes());
    buf[40..44].copy_from_slice(&0x00FF_0000u32.to_le_bytes());
    buf[44..48].copy_from_slice(&0x0000_FF00u32.to_le_bytes());
    buf[48..52].copy_from_slice(&0x0000_00FFu32.to_le_bytes());
    buf[52..56].copy_from_slice(&0xFF00_0000u32.to_le_bytes());
    buf[56..60].copy_from_slice(&0x7352_4742u32.to_le_bytes());

    let raw_rgba = img.as_raw();
    let row_stride = (width as usize) * 4;

    for y in 0..(height as usize) {
        let src_row_y = (height as usize) - 1 - y;
        let src_start = src_row_y * row_stride;
        let dst_start = header_size + y * row_stride;

        for x in 0..(width as usize) {
            let px_src = src_start + x * 4;
            let px_dst = dst_start + x * 4;
            let r = raw_rgba[px_src];
            let g = raw_rgba[px_src + 1];
            let b = raw_rgba[px_src + 2];
            let a = raw_rgba[px_src + 3];

            buf[px_dst] = b;
            buf[px_dst + 1] = g;
            buf[px_dst + 2] = r;
            buf[px_dst + 3] = a;
        }
    }

    Some(buf)
}

#[cfg(windows)]
#[expect(clippy::collapsible_if, reason = "nested clipboard entry extraction")]
fn serve_windows(offer: &ClipboardOffer) -> Result<(), ClipboardError> {
    use std::ffi::c_void;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};

    const CF_UNICODETEXT: u32 = 13;
    const CF_DIBV5: u32 = 17;

    unsafe {
        if OpenClipboard(None).is_err() {
            return Err(ClipboardError::transport(std::io::Error::other(
                "failed to open Windows clipboard",
            )));
        }
        let _ = EmptyClipboard();
    }

    if let Some(entry) = offer
        .entries
        .iter()
        .find(|e| e.mime == MIME_PNG || e.mime == MIME_JPEG)
    {
        if let Some(dib_data) = create_dibv5_buffer(&entry.data) {
            if let Ok(hglobal) = unsafe { GlobalAlloc(GMEM_MOVEABLE, dib_data.len()) } {
                let ptr = unsafe { GlobalLock(hglobal) };
                if !ptr.is_null() {
                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            dib_data.as_ptr().cast::<c_void>(),
                            ptr,
                            dib_data.len(),
                        );
                        let _ = GlobalUnlock(hglobal);
                        let _ = SetClipboardData(CF_DIBV5, Some(HANDLE(hglobal.0)));
                    }
                }
            }
        }
    }

    if let Some(entry) = offer.entries.iter().find(|e| e.mime == MIME_TEXT_PLAIN) {
        if let Ok(text) = std::str::from_utf8(&entry.data) {
            let utf16: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
            let byte_len = utf16.len() * 2;
            if let Ok(hglobal) = unsafe { GlobalAlloc(GMEM_MOVEABLE, byte_len) } {
                let ptr = unsafe { GlobalLock(hglobal) };
                if !ptr.is_null() {
                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            utf16.as_ptr().cast::<c_void>(),
                            ptr,
                            byte_len,
                        );
                        let _ = GlobalUnlock(hglobal);
                        let _ = SetClipboardData(CF_UNICODETEXT, Some(HANDLE(hglobal.0)));
                    }
                }
            }
        }
    }

    unsafe {
        let _ = CloseClipboard();
    }

    Ok(())
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "unit test assertions")]
mod tests {
    use super::*;

    #[test]
    fn create_dibv5_generates_valid_bmpv5_header() {
        let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([10, 20, 30, 255]));
        let mut png_bytes = Vec::new();
        img.write_with_encoder(image::codecs::png::PngEncoder::new(&mut png_bytes))
            .unwrap();

        let dib = create_dibv5_buffer(&png_bytes).unwrap();
        assert!(dib.len() >= 124 + 16);
        let header_size = u32::from_le_bytes(dib[0..4].try_into().unwrap());
        assert_eq!(header_size, 124);
        let width = i32::from_le_bytes(dib[4..8].try_into().unwrap());
        assert_eq!(width, 2);
        let height = i32::from_le_bytes(dib[8..12].try_into().unwrap());
        assert_eq!(height, 2);
    }
}
