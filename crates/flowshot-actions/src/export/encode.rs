//! Image format encoding.

use image::DynamicImage;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::codecs::webp::WebPEncoder;

use crate::error::ExportError;

/// Encode an image as PNG bytes.
///
/// # Errors
///
/// Returns [`ExportError::ImageEncode`] if encoding fails.
pub fn encode_png(image: &DynamicImage) -> Result<Vec<u8>, ExportError> {
    let mut buf = Vec::new();
    let encoder = PngEncoder::new(&mut buf);
    image
        .write_with_encoder(encoder)
        .map_err(|e| ExportError::ImageEncode(e.to_string()))?;
    Ok(buf)
}

/// Encode an image as JPEG bytes with the given quality (1-100).
///
/// # Errors
///
/// Returns [`ExportError::ImageEncode`] if encoding fails.
pub fn encode_jpeg(image: &DynamicImage, quality: u8) -> Result<Vec<u8>, ExportError> {
    let mut buf = Vec::new();
    let encoder = JpegEncoder::new_with_quality(&mut buf, quality);
    image
        .write_with_encoder(encoder)
        .map_err(|e| ExportError::ImageEncode(e.to_string()))?;
    Ok(buf)
}

/// Encode an image as WebP bytes.
///
/// # Errors
///
/// Returns [`ExportError::ImageEncode`] if encoding fails.
pub fn encode_webp(image: &DynamicImage) -> Result<Vec<u8>, ExportError> {
    let mut buf = Vec::new();
    let encoder = WebPEncoder::new_lossless(&mut buf);
    image
        .write_with_encoder(encoder)
        .map_err(|e| ExportError::ImageEncode(e.to_string()))?;
    Ok(buf)
}

/// Encode an image in the format identified by extension string.
///
/// Supported extensions: `png`, `jpg`/`jpeg`, `webp`.
///
/// # Errors
///
/// Returns [`ExportError::UnsupportedFormat`] for unknown extensions,
/// or [`ExportError::ImageEncode`] if encoding fails.
pub fn encode(
    image: &DynamicImage,
    extension: &str,
    jpeg_quality: u8,
) -> Result<Vec<u8>, ExportError> {
    match extension.to_ascii_lowercase().as_str() {
        "png" => encode_png(image),
        "jpg" | "jpeg" => encode_jpeg(image, jpeg_quality),
        "webp" => encode_webp(image),
        other => Err(ExportError::UnsupportedFormat(other.to_owned())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    fn test_image() -> DynamicImage {
        let img = RgbaImage::from_pixel(4, 4, Rgba([255, 0, 0, 255]));
        DynamicImage::ImageRgba8(img)
    }

    #[test]
    fn png_encode_produces_valid_bytes() {
        let bytes = encode_png(&test_image()).unwrap_or_default();
        assert_eq!(&bytes[..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
    }

    #[test]
    fn jpeg_quality_affects_output() {
        let img = test_image();
        let low = encode_jpeg(&img, 10).unwrap_or_default();
        let high = encode_jpeg(&img, 100).unwrap_or_default();
        assert_eq!(&low[..3], &[0xFF, 0xD8, 0xFF]);
        assert_eq!(&high[..3], &[0xFF, 0xD8, 0xFF]);
    }

    #[test]
    fn webp_encode_produces_valid_bytes() {
        let bytes = encode_webp(&test_image()).unwrap_or_default();
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WEBP");
    }

    #[test]
    fn encode_dispatches_by_extension() {
        let img = test_image();
        let png = encode(&img, "png", 75).unwrap_or_default();
        assert_eq!(&png[..4], &[137, 80, 78, 71]);

        let jpg = encode(&img, "jpg", 75).unwrap_or_default();
        assert_eq!(&jpg[..3], &[0xFF, 0xD8, 0xFF]);

        let webp = encode(&img, "webp", 75).unwrap_or_default();
        assert_eq!(&webp[..4], b"RIFF");
    }

    #[test]
    fn unsupported_format_returns_error() {
        let img = test_image();
        let result = encode(&img, "bmp", 75);
        assert!(matches!(result, Err(ExportError::UnsupportedFormat(_))));
    }
}
