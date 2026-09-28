//! Reply-metadata parsing and raw-payload assembly for the `ScreenShot2`
//! wire contract.
//!
//! The metadata vardict keys and the `QImage::Format` enum values are pinned
//! to the fetched upstream sources cited in [`crate::kwin`]: the reply
//! carries `{type:"raw", format, width, height, stride, scale}` and MAY carry
//! additive keys (`screen`, `windowId`, future additions) which the parser
//! MUST ignore. The pipe payload is raw `QImage` bits - never an encoded
//! image - exactly `stride * height` bytes.

use std::collections::HashMap;

use bytes::BytesMut;
use flowshot_capture::{FrameBuffer, FrameFormat};
use flowshot_core::geometry::Transform;
use zbus::zvariant::Value;

use super::error::{DecodeError, KwinError};

/// `QImage::Format_RGB32`: little-endian `0xffRRGGBB`, bytes B, G, R, X.
const QIMAGE_RGB32: u32 = 4;
/// `QImage::Format_ARGB32`: little-endian `0xAARRGGBB`, bytes B, G, R, A.
const QIMAGE_ARGB32: u32 = 5;
/// `QImage::Format_ARGB32_Premultiplied`.
const QIMAGE_ARGB32_PREMULTIPLIED: u32 = 6;
/// `QImage::Format_RGBX8888`: bytes R, G, B, X (X undefined).
const QIMAGE_RGBX8888: u32 = 16;
/// `QImage::Format_RGBA8888`: bytes R, G, B, A.
const QIMAGE_RGBA8888: u32 = 17;
/// `QImage::Format_RGBA8888_Premultiplied`.
const QIMAGE_RGBA8888_PREMULTIPLIED: u32 = 18;

/// Sanity bound for `stride * height`: a 16K x 16K 32-bpp capture is 1 GiB,
/// beyond any real output; larger announcements are corrupt metadata and
/// must never reach an allocation (a huge `Vec` capacity aborts).
const MAX_PAYLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// How a `QImage::Format` maps onto the shared pixel vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PixelLayout {
    /// The shared frame format with the identical byte order.
    pub format: FrameFormat,
    /// Whether the format's fourth byte is undefined padding that must be
    /// forced to 255 (`Format_RGBX8888`; the portal `ScreenCast` `RGBx`
    /// precedent - consumers pass an `A` byte straight through).
    pub force_opaque_alpha: bool,
}

/// Maps a `QImage::Format` enum value onto the shared pixel vocabulary.
///
/// The premultiplied variants map onto their straight-alpha equivalents:
/// screen content is opaque (alpha 255), where premultiplication is the
/// identity; a non-opaque pixel would stay premultiplied - a documented
/// degradation, traced at the call site.
///
/// # Errors
///
/// [`DecodeError::UnknownFormat`] for any value outside the 32-bpp set the
/// v1 [`FrameFormat`] vocabulary can represent losslessly.
pub(crate) fn qimage_layout(qimage_format: u32) -> Result<PixelLayout, DecodeError> {
    let layout = match qimage_format {
        QIMAGE_RGB32 => PixelLayout {
            format: FrameFormat::Xrgb8888,
            force_opaque_alpha: false,
        },
        QIMAGE_ARGB32 | QIMAGE_ARGB32_PREMULTIPLIED => PixelLayout {
            format: FrameFormat::Argb8888,
            force_opaque_alpha: false,
        },
        QIMAGE_RGBX8888 => PixelLayout {
            format: FrameFormat::Rgba8888,
            force_opaque_alpha: true,
        },
        QIMAGE_RGBA8888 | QIMAGE_RGBA8888_PREMULTIPLIED => PixelLayout {
            format: FrameFormat::Rgba8888,
            force_opaque_alpha: false,
        },
        _ => {
            return Err(DecodeError::UnknownFormat {
                format: qimage_format,
            });
        }
    };
    Ok(layout)
}

/// The parsed `ScreenShot2` reply metadata.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RawFrameMeta {
    /// The mapped shared pixel format.
    pub layout: PixelLayout,
    /// The raw `QImage::Format` value (diagnostics).
    pub qimage_format: u32,
    /// Image width in physical pixels.
    pub width: u32,
    /// Image height in physical pixels.
    pub height: u32,
    /// Bytes per row (may exceed `width * 4`).
    pub stride: u32,
    /// The capture's device pixel ratio.
    pub scale: f64,
    /// The exact payload byte count the pipe must deliver.
    pub expected_bytes: usize,
    /// The additive `screen` key when present (diagnostics only).
    pub screen: Option<ScreenName>,
}

/// The metadata's optional `screen` value, kept for diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScreenName(pub String);

/// Parses the reply metadata vardict, ignoring every unknown key.
///
/// # Errors
///
/// [`DecodeError`] when a required key is missing, carries the wrong value
/// type, announces a non-`raw` payload, an unmappable `QImage` format, or an
/// inconsistent/implausible geometry.
pub(crate) fn parse_metadata(
    values: &HashMap<String, Value<'_>>,
) -> Result<RawFrameMeta, DecodeError> {
    let kind = str_value(values, "type")?;
    if kind != "raw" {
        return Err(DecodeError::UnsupportedType { found: kind });
    }
    let qimage_format = u32_value(values, "format")?;
    let layout = qimage_layout(qimage_format)?;
    let width = u32_value(values, "width")?;
    let height = u32_value(values, "height")?;
    let stride = u32_value(values, "stride")?;
    let scale = f64_value(values, "scale")?;
    let expected_bytes = payload_size(width, height, stride)?;
    let screen = match str_value(values, "screen") {
        Ok(name) => Some(ScreenName(name)),
        Err(DecodeError::MissingKey { .. }) => None,
        Err(error) => return Err(error),
    };
    Ok(RawFrameMeta {
        layout,
        qimage_format,
        width,
        height,
        stride,
        scale,
        expected_bytes,
        screen,
    })
}

/// Validates the announced geometry and computes the exact payload size.
///
/// # Errors
///
/// [`DecodeError::InvalidGeometry`] for a zero dimension or a stride narrower
/// than `width * 4`, [`DecodeError::PayloadTooLarge`] beyond the sanity
/// bound, and [`DecodeError::InvalidGeometry`] on `usize` overflow.
fn payload_size(width: u32, height: u32, stride: u32) -> Result<usize, DecodeError> {
    let invalid = || DecodeError::InvalidGeometry {
        width,
        height,
        stride,
    };
    if width == 0 || height == 0 {
        return Err(invalid());
    }
    let row_pixels = u64::from(width).checked_mul(4).ok_or_else(invalid)?;
    if u64::from(stride) < row_pixels {
        return Err(invalid());
    }
    let bytes = u64::from(stride)
        .checked_mul(u64::from(height))
        .ok_or_else(invalid)?;
    if bytes > MAX_PAYLOAD_BYTES {
        return Err(DecodeError::PayloadTooLarge { bytes });
    }
    usize::try_from(bytes).map_err(|_| invalid())
}

/// Assembles the [`FrameBuffer`] from the validated metadata and the raw
/// pipe payload, forcing undefined alpha bytes to 255 when the format
/// requires it.
///
/// # Errors
///
/// [`DecodeError::TruncatedPayload`] when `data` is shorter than the
/// metadata's `stride * height`, and [`DecodeError::InvalidGeometry`] when
/// the stride overflows `u32` arithmetic.
pub(crate) fn frame_buffer(
    meta: &RawFrameMeta,
    mut data: Vec<u8>,
) -> Result<FrameBuffer, DecodeError> {
    if data.len() < meta.expected_bytes {
        return Err(DecodeError::TruncatedPayload {
            expected: meta.expected_bytes,
            received: data.len(),
        });
    }
    if meta.layout.force_opaque_alpha {
        force_alpha_opaque(&mut data, meta);
    }
    Ok(FrameBuffer {
        data: BytesMut::from(data.as_slice()),
        width: meta.width,
        height: meta.height,
        stride: meta.stride,
        format: meta.layout.format,
    })
}

/// Sets every pixel's alpha byte to 255 (row padding is left untouched).
fn force_alpha_opaque(data: &mut [u8], meta: &RawFrameMeta) {
    let stride = usize::try_from(meta.stride).unwrap_or(usize::MAX);
    let width = usize::try_from(meta.width).unwrap_or(usize::MAX);
    let height = usize::try_from(meta.height).unwrap_or(usize::MAX);
    for row in 0..height {
        for column in 0..width {
            let offset = row.saturating_mul(stride).saturating_add(column * 4 + 3);
            if let Some(byte) = data.get_mut(offset) {
                *byte = 255;
            }
        }
    }
}

/// Inverse-remaps an upright (post-transform) buffer back to the output's
/// native orientation, per the shared [`Frame`](flowshot_capture::Frame)
/// contract - `KWin` renders areas as displayed, like the `grim`-based
/// portal composite (the same normalization the ICC and portal backends
/// apply).
///
/// # Errors
///
/// [`KwinError::Internal`] on dimension conversion failures and
/// [`KwinError::Geometry`] when the remap rejects the buffers.
pub(crate) fn to_native_orientation(
    buffer: FrameBuffer,
    transform: Transform,
) -> Result<FrameBuffer, KwinError> {
    if transform == Transform::Normal {
        return Ok(buffer);
    }
    let inverse = transform.inverse();
    let (Ok(width), Ok(height)) = (
        usize::try_from(buffer.width),
        usize::try_from(buffer.height),
    ) else {
        return Err(KwinError::Internal("buffer dimensions overflow usize"));
    };
    let mut data = vec![0u8; buffer.data.len()];
    inverse.remap_buffer(&buffer.data, &mut data, width, height, 4)?;
    let (width, height) = if inverse.swaps_dimensions() {
        (buffer.height, buffer.width)
    } else {
        (buffer.width, buffer.height)
    };
    let stride = width
        .checked_mul(4)
        .ok_or(KwinError::Internal("native stride overflows u32"))?;
    Ok(FrameBuffer {
        data: BytesMut::from(data.as_slice()),
        width,
        height,
        stride,
        format: buffer.format,
    })
}

/// Peels one layer of `v`-in-`v` nesting: a `D-Bus` variant entry may
/// deserialize as the inner value directly or wrapped, depending on the
/// signature path `zvariant` took.
fn inner<'a, 'v>(value: &'a Value<'v>) -> &'a Value<'v> {
    match value {
        Value::Value(nested) => inner(nested),
        other => other,
    }
}

fn u32_value(values: &HashMap<String, Value<'_>>, key: &'static str) -> Result<u32, DecodeError> {
    match inner(value_at(values, key)?) {
        Value::U32(value) => Ok(*value),
        _ => Err(DecodeError::InvalidKeyType { key }),
    }
}

fn f64_value(values: &HashMap<String, Value<'_>>, key: &'static str) -> Result<f64, DecodeError> {
    match inner(value_at(values, key)?) {
        Value::F64(value) => Ok(*value),
        _ => Err(DecodeError::InvalidKeyType { key }),
    }
}

fn str_value(
    values: &HashMap<String, Value<'_>>,
    key: &'static str,
) -> Result<String, DecodeError> {
    match inner(value_at(values, key)?) {
        Value::Str(value) => Ok(value.to_string()),
        _ => Err(DecodeError::InvalidKeyType { key }),
    }
}

fn value_at<'v>(
    values: &'v HashMap<String, Value<'v>>,
    key: &'static str,
) -> Result<&'v Value<'v>, DecodeError> {
    values.get(key).ok_or(DecodeError::MissingKey { key })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    // Fixture scales and sizes are exact integral literals (geometry-notepad
    // convention), so strict comparison is the intended assertion.
    #![allow(clippy::float_cmp)]

    use super::*;

    fn meta_values(
        format: u32,
        width: u32,
        height: u32,
        stride: u32,
    ) -> HashMap<String, Value<'static>> {
        HashMap::from([
            ("type".to_owned(), Value::Str("raw".to_owned().into())),
            ("format".to_owned(), Value::U32(format)),
            ("width".to_owned(), Value::U32(width)),
            ("height".to_owned(), Value::U32(height)),
            ("stride".to_owned(), Value::U32(stride)),
            ("scale".to_owned(), Value::F64(1.0)),
        ])
    }

    #[test]
    fn qimage_format_table_maps_the_32bpp_set() {
        assert_eq!(
            qimage_layout(QIMAGE_RGB32).unwrap(),
            PixelLayout {
                format: FrameFormat::Xrgb8888,
                force_opaque_alpha: false
            }
        );
        assert_eq!(
            qimage_layout(QIMAGE_ARGB32).unwrap().format,
            FrameFormat::Argb8888
        );
        assert_eq!(
            qimage_layout(QIMAGE_ARGB32_PREMULTIPLIED).unwrap().format,
            FrameFormat::Argb8888
        );
        assert_eq!(
            qimage_layout(QIMAGE_RGBX8888).unwrap(),
            PixelLayout {
                format: FrameFormat::Rgba8888,
                force_opaque_alpha: true
            }
        );
        assert_eq!(
            qimage_layout(QIMAGE_RGBA8888).unwrap().format,
            FrameFormat::Rgba8888
        );
        assert_eq!(
            qimage_layout(QIMAGE_RGBA8888_PREMULTIPLIED).unwrap().format,
            FrameFormat::Rgba8888
        );
    }

    #[test]
    fn unknown_qimage_format_is_a_typed_decode_error() {
        for format in [0u32, 1, 3, 7, 13, 19, 25, 29, 999, u32::MAX] {
            match qimage_layout(format).unwrap_err() {
                DecodeError::UnknownFormat { format: reported } => {
                    assert_eq!(reported, format);
                }
                other => panic!("expected UnknownFormat, got {other:?}"),
            }
        }
    }

    #[test]
    fn full_metadata_parses_and_ignores_unknown_keys() {
        let mut values = meta_values(QIMAGE_ARGB32, 4, 3, 16);
        values.insert("screen".to_owned(), Value::Str("DP-1".to_owned().into()));
        values.insert("windowId".to_owned(), Value::Str("uuid".to_owned().into()));
        values.insert("from-the-future".to_owned(), Value::U64(42));
        let meta = parse_metadata(&values).unwrap();
        assert_eq!(meta.width, 4);
        assert_eq!(meta.height, 3);
        assert_eq!(meta.stride, 16);
        assert_eq!(meta.scale, 1.0);
        assert_eq!(meta.expected_bytes, 48);
        assert_eq!(meta.layout.format, FrameFormat::Argb8888);
        assert_eq!(meta.screen, Some(ScreenName("DP-1".to_owned())));
    }

    #[test]
    fn non_raw_type_is_a_typed_decode_error() {
        let mut values = meta_values(QIMAGE_ARGB32, 4, 3, 16);
        values.insert("type".to_owned(), Value::Str("png".to_owned().into()));
        match parse_metadata(&values).unwrap_err() {
            DecodeError::UnsupportedType { found } => assert_eq!(found, "png"),
            other => panic!("expected UnsupportedType, got {other:?}"),
        }
    }

    #[test]
    fn missing_and_mistyped_keys_are_typed_decode_errors() {
        let mut values = meta_values(QIMAGE_ARGB32, 4, 3, 16);
        values.remove("stride");
        assert!(matches!(
            parse_metadata(&values).unwrap_err(),
            DecodeError::MissingKey { key: "stride" }
        ));
        let mut values = meta_values(QIMAGE_ARGB32, 4, 3, 16);
        values.insert("width".to_owned(), Value::Str("4".to_owned().into()));
        assert!(matches!(
            parse_metadata(&values).unwrap_err(),
            DecodeError::InvalidKeyType { key: "width" }
        ));
    }

    #[test]
    fn variant_nested_metadata_values_unwrap() {
        let mut values = meta_values(QIMAGE_ARGB32, 4, 3, 16);
        values.insert("width".to_owned(), Value::Value(Box::new(Value::U32(4))));
        assert_eq!(parse_metadata(&values).unwrap().width, 4);
    }

    #[test]
    fn inconsistent_geometry_is_rejected_before_any_allocation() {
        for (width, height, stride) in [(0u32, 3u32, 16u32), (4, 0, 16), (4, 3, 12)] {
            let values = meta_values(QIMAGE_ARGB32, width, height, stride);
            assert!(
                matches!(
                    parse_metadata(&values).unwrap_err(),
                    DecodeError::InvalidGeometry { .. }
                ),
                "{width}x{height} stride {stride} must be rejected"
            );
        }
        let values = meta_values(QIMAGE_ARGB32, 32_768, 32_768, 131_072);
        assert!(matches!(
            parse_metadata(&values).unwrap_err(),
            DecodeError::PayloadTooLarge { .. }
        ));
    }

    #[test]
    fn truncated_payload_is_a_typed_decode_error() {
        let meta = parse_metadata(&meta_values(QIMAGE_ARGB32, 4, 3, 16)).unwrap();
        match frame_buffer(&meta, vec![7u8; 40]).unwrap_err() {
            DecodeError::TruncatedPayload { expected, received } => {
                assert_eq!(expected, 48);
                assert_eq!(received, 40);
            }
            other => panic!("expected TruncatedPayload, got {other:?}"),
        }
    }

    #[test]
    fn rgbx_payload_gets_opaque_alpha_forced() {
        let meta = parse_metadata(&meta_values(QIMAGE_RGBX8888, 2, 1, 8)).unwrap();
        let buffer = frame_buffer(&meta, vec![1, 2, 3, 0, 5, 6, 7, 0]).unwrap();
        assert_eq!(buffer.format, FrameFormat::Rgba8888);
        assert_eq!(buffer.pixel(0, 0), Some([1, 2, 3, 255]));
        assert_eq!(buffer.pixel(1, 0), Some([5, 6, 7, 255]));
    }

    #[test]
    fn padded_stride_keeps_padding_untouched() {
        let meta = parse_metadata(&meta_values(QIMAGE_RGBX8888, 1, 2, 8)).unwrap();
        let buffer =
            frame_buffer(&meta, vec![1, 2, 3, 0, 9, 9, 9, 9, 5, 6, 7, 0, 9, 9, 9, 9]).unwrap();
        assert_eq!(buffer.stride, 8);
        assert_eq!(&buffer.data[4..8], &[9, 9, 9, 9]);
        assert_eq!(buffer.pixel(0, 1), Some([5, 6, 7, 255]));
    }

    #[test]
    fn rotated_upright_buffer_inverse_remaps_to_native() {
        // Given a Rot90 output whose upright 4x3 image has a red BOTTOM row,
        let upright = FrameBuffer {
            data: BytesMut::from(
                [[0u8, 255, 0, 255].repeat(8), [255u8, 0, 0, 255].repeat(4)]
                    .concat()
                    .as_slice(),
            ),
            width: 4,
            height: 3,
            stride: 16,
            format: FrameFormat::Rgba8888,
        };
        // When inverse-remapping to native orientation,
        let native = to_native_orientation(upright, Transform::Rot90).unwrap();
        // Then the buffer is native 3x4 with the red row as the LEFT column
        // (the stitch.rs Rot90 fixture discipline, mirrored).
        assert_eq!((native.width, native.height), (3, 4));
        for y in 0..4 {
            assert_eq!(native.pixel(0, y), Some([255, 0, 0, 255]));
            assert_eq!(native.pixel(1, y), Some([0, 255, 0, 255]));
        }
    }

    #[test]
    fn normal_transform_borrows_without_remap() {
        let buffer = FrameBuffer {
            data: BytesMut::from(&[7u8; 48][..]),
            width: 4,
            height: 3,
            stride: 16,
            format: FrameFormat::Argb8888,
        };
        assert_eq!(
            to_native_orientation(buffer.clone(), Transform::Normal).unwrap(),
            buffer
        );
    }
}
