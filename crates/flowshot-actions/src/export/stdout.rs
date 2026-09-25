//! Stdout output modes: raw PNG bytes and geometry string.

use std::io::Write;

use image::DynamicImage;

use crate::error::ExportError;

/// Write the image as raw PNG bytes to the given writer.
///
/// # Errors
///
/// Returns [`ExportError`] if encoding or writing fails.
pub fn write_raw_png<W: Write>(image: &DynamicImage, writer: &mut W) -> Result<(), ExportError> {
    let bytes = super::encode_png(image)?;
    writer.write_all(&bytes)?;
    Ok(())
}

/// Format a geometry string as `WxH+X+Y`.
#[must_use]
pub fn format_geometry(x: i32, y: i32, width: u32, height: u32) -> String {
    format!("{width}x{height}+{x}+{y}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    fn test_image() -> DynamicImage {
        let img = RgbaImage::from_pixel(2, 2, Rgba([0, 255, 0, 255]));
        DynamicImage::ImageRgba8(img)
    }

    #[test]
    fn raw_png_starts_with_magic_bytes() {
        let mut buf = Vec::new();
        write_raw_png(&test_image(), &mut buf).ok();
        assert_eq!(&buf[..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
    }

    #[test]
    fn geometry_string_format() {
        let s = format_geometry(100, 200, 1920, 1080);
        assert_eq!(s, "1920x1080+100+200");
    }

    #[test]
    fn geometry_regex_match() {
        let s = format_geometry(0, 0, 800, 600);
        let parts: Vec<&str> = s.split(['x', '+']).collect();
        assert_eq!(parts.len(), 4);
        assert!(parts.iter().all(|p| p.parse::<u32>().is_ok()));
    }
}
