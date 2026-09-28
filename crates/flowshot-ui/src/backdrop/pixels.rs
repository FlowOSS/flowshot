//! Pixel preparation for the frozen-frame backdrop.
//!
//! Captured [`Frame`] buffers arrive in a compositor pixel format
//! ([`FrameFormat`]), with possible row-stride padding, in the output's
//! NATIVE pre-transform orientation (the shared frame contract). The
//! renderer wants tight, row-major, `RGBA8888` pixels in post-transform
//! (upright) layout orientation, so preparation runs three steps:
//! stride-tighten + format-convert, then [`Transform::remap_buffer`] to
//! upright, then a dimension guard against [`OutputInfo::buffer_size`].
//!
//! Cursor sprites additionally premultiply their straight alpha INTO the
//! renderer's premultiplied pipeline contract - in encoded sRGB space via
//! the exact decode/multiply/re-encode round trip, so composited cursor
//! edges match the GPU blend (`src + dst * (1 - src_a)`) without fringing.

use flowshot_capture::{Frame, FrameBuffer, FrameFormat};
use flowshot_core::geometry::{OutputInfo, Transform};

use crate::error::UiError;
use crate::render::{linear_to_srgb, srgb_to_linear};

use super::CursorSprite;

/// Tight, upright, renderer-ready `RGBA8888` pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedTexture {
    /// Row-major RGBA bytes (`width * height * 4`), post-transform
    /// (upright) orientation.
    pub data: Vec<u8>,
    /// Width in physical pixels (upright orientation).
    pub width: u32,
    /// Height in physical pixels (upright orientation).
    pub height: u32,
}

/// Converts one pixel of any v1 frame format to byte order R, G, B, A.
///
/// Mirrors the shared region-stitch conversion in the platform capture
/// crate (which the purity gate forbids importing here); the table is the
/// little-endian shared-memory convention:
/// `Xrgb8888` = bytes B,G,R,X (opaque), `Argb8888` = bytes B,G,R,A.
#[must_use]
pub(super) fn to_rgba(format: FrameFormat, pixel: [u8; 4]) -> [u8; 4] {
    match format {
        FrameFormat::Xrgb8888 => [pixel[2], pixel[1], pixel[0], 255],
        FrameFormat::Argb8888 => [pixel[2], pixel[1], pixel[0], pixel[3]],
        FrameFormat::Rgba8888 => pixel,
    }
}

/// Prepares one output's captured frame for texture upload.
///
/// # Errors
///
/// [`UiError::BackdropFrameMismatch`] when the buffer dimensions disagree
/// with the output's native physical size, [`UiError::TextureDataLength`]
/// when the buffer holds fewer bytes than its stride geometry requires, and
/// [`UiError::Geometry`] when the transform remap rejects the buffer.
pub fn prepare_output_texture(
    frame: &Frame,
    output: &OutputInfo,
) -> Result<PreparedTexture, UiError> {
    let native = (
        u32::try_from(output.physical_size.width.0),
        u32::try_from(output.physical_size.height.0),
    );
    let (Ok(expected_width), Ok(expected_height)) = native else {
        return Err(mismatch(frame, output, 0, 0));
    };
    if frame.buffer.width != expected_width || frame.buffer.height != expected_height {
        return Err(mismatch(frame, output, expected_width, expected_height));
    }
    let tight = tighten_to_rgba(&frame.buffer).ok_or(UiError::TextureDataLength {
        width: frame.buffer.width,
        height: frame.buffer.height,
        expected: row_end(expected_width, expected_height, frame.buffer.stride).unwrap_or(0),
        actual: frame.buffer.data.len(),
    })?;
    if output.transform != Transform::Normal {
        return remap_to_upright(&tight, expected_width, expected_height, output);
    }
    Ok(PreparedTexture {
        data: tight,
        width: expected_width,
        height: expected_height,
    })
}

/// Premultiplies a cursor sprite's straight alpha for the renderer's
/// premultiplied image pipeline; `None` on a dimension/data mismatch.
#[must_use]
pub(super) fn prepare_cursor_texture(sprite: &CursorSprite) -> Option<Vec<u8>> {
    let width = usize::try_from(sprite.width).ok()?;
    let height = usize::try_from(sprite.height).ok()?;
    if width == 0 || height == 0 {
        return None;
    }
    let expected = width.checked_mul(height)?.checked_mul(4)?;
    if sprite.rgba.len() != expected {
        return None;
    }
    let mut data = sprite.rgba.clone();
    for pixel in data.as_chunks_mut::<4>().0 {
        let alpha = f32::from(pixel[3]) / 255.0;
        for channel in pixel.iter_mut().take(3) {
            let linear = srgb_to_linear(f32::from(*channel) / 255.0) * alpha;
            *channel = unit_to_byte(linear_to_srgb(linear));
        }
    }
    Some(data)
}

fn unit_to_byte(unit: f32) -> u8 {
    let scaled = (unit.clamp(0.0, 1.0) * 255.0).round();
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to [0, 1] and rounded, so the value is a whole byte"
    )]
    let byte = scaled as u8;
    byte
}

fn mismatch(
    frame: &Frame,
    output: &OutputInfo,
    expected_width: u32,
    expected_height: u32,
) -> UiError {
    UiError::BackdropFrameMismatch {
        connector: output.connector.clone(),
        actual_width: frame.buffer.width,
        actual_height: frame.buffer.height,
        expected_width,
        expected_height,
    }
}

fn row_end(width: u32, height: u32, stride: u32) -> Option<usize> {
    let rows = usize::try_from(height.checked_sub(1)?).ok()?;
    let stride = usize::try_from(stride).ok()?;
    let row = usize::try_from(width).ok()?.checked_mul(4)?;
    rows.checked_mul(stride)?.checked_add(row)
}

/// Stride-tightens and format-converts a frame buffer into tight `RGBA8888`.
fn tighten_to_rgba(buffer: &FrameBuffer) -> Option<Vec<u8>> {
    let width = usize::try_from(buffer.width).ok()?;
    let height = usize::try_from(buffer.height).ok()?;
    let stride = usize::try_from(buffer.stride).ok()?;
    if width == 0 || height == 0 || stride < width.checked_mul(4)? {
        return None;
    }
    let row_bytes = width.checked_mul(4)?;
    let needed = (height - 1).checked_mul(stride)?.checked_add(row_bytes)?;
    if buffer.data.len() < needed {
        return None;
    }
    let mut data = Vec::with_capacity(width.checked_mul(height)?.checked_mul(4)?);
    for row in 0..height {
        let start = row.checked_mul(stride)?;
        let bytes = buffer.data.get(start..start.checked_add(row_bytes)?)?;
        for pixel in bytes.as_chunks::<4>().0 {
            data.extend_from_slice(&to_rgba(buffer.format, *pixel));
        }
    }
    Some(data)
}

fn remap_to_upright(
    tight: &[u8],
    width: u32,
    height: u32,
    output: &OutputInfo,
) -> Result<PreparedTexture, UiError> {
    let mut upright = vec![0u8; tight.len()];
    output
        .transform
        .remap_buffer(
            tight,
            &mut upright,
            usize::try_from(width).unwrap_or(usize::MAX),
            usize::try_from(height).unwrap_or(usize::MAX),
            4,
        )
        .map_err(UiError::Geometry)?;
    let size = output.buffer_size();
    Ok(PreparedTexture {
        data: upright,
        width: u32::try_from(size.width.0).unwrap_or(u32::MAX),
        height: u32::try_from(size.height.0).unwrap_or(u32::MAX),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use bytes::BytesMut;
    use flowshot_capture::{FrameFormat, OutputRef};
    use flowshot_core::geometry::{LogicalRect, PhysicalPoint, PhysicalSize};

    use super::*;

    fn output(transform: Transform, physical: (i32, i32)) -> OutputInfo {
        let size = PhysicalSize::from_raw(physical.0, physical.1);
        let logical = transform.apply_to_size(size);
        OutputInfo::new(
            "DP-1",
            "DP-1",
            LogicalRect::from_raw(
                0.0,
                0.0,
                f64::from(logical.width.0),
                f64::from(logical.height.0),
            ),
            size,
            1.0,
            transform,
        )
        .expect("valid fixture output")
    }

    fn frame(width: u32, height: u32, stride: u32, format: FrameFormat, data: &[u8]) -> Frame {
        Frame {
            buffer: FrameBuffer {
                data: BytesMut::from(data),
                width,
                height,
                stride,
                format,
            },
            output: OutputRef::Connector("DP-1".to_owned()),
            scale: 1.0,
            transform: Transform::Normal,
        }
    }

    #[test]
    fn to_rgba_converts_every_v1_format() {
        // Bytes B=1, G=2, R=3, X/A=4 in memory.
        let bytes = [1, 2, 3, 4];
        assert_eq!(to_rgba(FrameFormat::Xrgb8888, bytes), [3, 2, 1, 255]);
        assert_eq!(to_rgba(FrameFormat::Argb8888, bytes), [3, 2, 1, 4]);
        assert_eq!(to_rgba(FrameFormat::Rgba8888, bytes), bytes);
    }

    #[test]
    fn tighten_strips_stride_padding_and_converts() {
        // 2x1 XRGB with stride 12: 8 payload bytes + 4 garbage padding bytes.
        let mut data = vec![10, 20, 30, 0, 40, 50, 60, 0];
        data.extend_from_slice(&[99, 99, 99, 99]);
        let buffer = FrameBuffer {
            data: BytesMut::from(data.as_slice()),
            width: 2,
            height: 1,
            stride: 12,
            format: FrameFormat::Xrgb8888,
        };
        let tight = tighten_to_rgba(&buffer).expect("valid buffer");
        assert_eq!(tight, vec![30, 20, 10, 255, 60, 50, 40, 255]);
    }

    #[test]
    fn tighten_rejects_short_and_degenerate_buffers() {
        let short = FrameBuffer {
            data: BytesMut::from(&[0u8; 4][..]),
            width: 2,
            height: 2,
            stride: 8,
            format: FrameFormat::Rgba8888,
        };
        assert!(tighten_to_rgba(&short).is_none());
        let zero = FrameBuffer {
            data: BytesMut::new(),
            width: 0,
            height: 4,
            stride: 0,
            format: FrameFormat::Rgba8888,
        };
        assert!(tighten_to_rgba(&zero).is_none());
    }

    #[test]
    fn prepare_passes_rgba_through_untouched() {
        let data = vec![1u8, 2, 3, 4, 5, 6, 7, 8];
        let prepared = prepare_output_texture(
            &frame(2, 1, 8, FrameFormat::Rgba8888, &data),
            &output(Transform::Normal, (2, 1)),
        )
        .expect("valid frame");
        assert_eq!(prepared.data, data);
        assert_eq!((prepared.width, prepared.height), (2, 1));
    }

    #[test]
    fn prepare_remaps_rotated_frame_to_upright() {
        // Native 2x3 (w x h), Rot90: map_point sends native (0, y) to
        // upright (y, last_x) - the left column becomes the bottom row.
        let red = [255u8, 0, 0, 255];
        let green = [0u8, 255, 0, 255];
        let mut data = Vec::new();
        for _y in 0..3 {
            for x in 0..2 {
                data.extend_from_slice(if x == 0 { &red } else { &green });
            }
        }
        let mut native = frame(2, 3, 8, FrameFormat::Rgba8888, &data);
        native.transform = Transform::Rot90;
        let prepared = prepare_output_texture(&native, &output(Transform::Rot90, (2, 3)))
            .expect("valid frame");
        assert_eq!((prepared.width, prepared.height), (3, 2));
        let pixel = |x: usize, y: usize| {
            let offset = (y * 3 + x) * 4;
            [
                prepared.data[offset],
                prepared.data[offset + 1],
                prepared.data[offset + 2],
                prepared.data[offset + 3],
            ]
        };
        assert_eq!(pixel(0, 1), red);
        assert_eq!(pixel(2, 1), red);
        assert_eq!(pixel(0, 0), green);
    }

    #[test]
    fn prepare_guards_frame_dimensions_against_output_geometry() {
        let data = vec![0u8; 16];
        let error = prepare_output_texture(
            &frame(4, 4, 16, FrameFormat::Rgba8888, &data),
            &output(Transform::Normal, (2, 2)),
        )
        .expect_err("mismatched frame");
        assert!(matches!(
            error,
            UiError::BackdropFrameMismatch {
                actual_width: 4,
                actual_height: 4,
                expected_width: 2,
                expected_height: 2,
                ..
            }
        ));
    }

    #[test]
    fn prepare_rejects_short_frame_data() {
        let error = prepare_output_texture(
            &frame(2, 2, 8, FrameFormat::Rgba8888, &[0u8; 8]),
            &output(Transform::Normal, (2, 2)),
        )
        .expect_err("short data");
        assert!(matches!(error, UiError::TextureDataLength { .. }));
    }

    #[test]
    fn cursor_premultiply_matches_the_blend_contract() {
        let sprite = CursorSprite {
            // Opaque white, half-alpha white, transparent black.
            rgba: vec![255, 255, 255, 255, 255, 255, 255, 128, 0, 0, 0, 0],
            width: 3,
            height: 1,
            hotspot: PhysicalPoint::zero(),
        };
        let data = prepare_cursor_texture(&sprite).expect("valid sprite");
        assert_eq!(&data[0..4], &[255, 255, 255, 255]);
        // Half-alpha white premultiplies in LINEAR light (the pipeline's
        // contract): stored = srgb_encode(0.5 linear) = 188, so the hardware
        // sRGB decode yields exactly 0.5 for the premultiplied blend.
        assert_eq!(&data[4..8], &[188, 188, 188, 128]);
        assert_eq!(&data[8..12], &[0, 0, 0, 0]);
    }

    #[test]
    fn cursor_preparation_rejects_bad_geometry() {
        let bad_length = CursorSprite {
            rgba: vec![0u8; 7],
            width: 2,
            height: 1,
            hotspot: PhysicalPoint::zero(),
        };
        assert!(prepare_cursor_texture(&bad_length).is_none());
        let zero = CursorSprite {
            rgba: vec![],
            width: 0,
            height: 4,
            hotspot: PhysicalPoint::zero(),
        };
        assert!(prepare_cursor_texture(&zero).is_none());
    }
}
