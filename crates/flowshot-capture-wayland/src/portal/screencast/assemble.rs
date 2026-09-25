//! Frame normalization for captured portal screencast streams: orientation
//! decision, stride validation, `RGBx` padding, and the inverse remap into
//! the shared [`Frame`](flowshot_capture::Frame) contract.

use bytes::BytesMut;
use flowshot_capture::{Frame, FrameBuffer, OutputRef};
use flowshot_core::geometry::{OutputInfo, Transform};

use super::super::composite::to_native_orientation;
use super::super::error::{PortalErrorKind, PortalScreenCastError};
use super::super::pipewire::RawPwFrame;
use super::super::streams::{Orientation, orientation_of};

/// Normalizes one raw stream frame into the shared [`Frame`] contract:
/// stride-padded data validated against the frame geometry, `RGBx` padding
/// forced opaque, upright buffers inverse-remapped to native orientation.
///
/// # Errors
///
/// [`PortalErrorKind::BufferSizeMismatch`] when the frame matches neither
/// the output's native nor post-transform size,
/// [`PortalErrorKind::IncompleteFrame`] when the data is shorter than the
/// geometry claims, and [`PortalErrorKind::Geometry`] when a remap rejects
/// the buffer.
pub(super) fn assemble_frame(
    output: &OutputInfo,
    raw: RawPwFrame,
) -> Result<Frame, PortalScreenCastError> {
    let orientation = orientation_of((raw.width, raw.height), output)?;
    if orientation == Orientation::Native
        && output.transform != Transform::Normal
        && output.physical_size == output.buffer_size()
    {
        // Rot180/flipped outputs have equal native and upright dimensions:
        // XDPH delivers native (assumed here), mutter would deliver upright.
        // Hardware-pending ambiguity, traced for diagnosis.
        tracing::warn!(
            output = %output.connector,
            transform = ?output.transform,
            "portal stream orientation is ambiguous for this transform; assuming native \
             (hardware-pending verification)"
        );
    }
    let min_bytes = stride_floor(&raw)?;
    if raw.data.len() < min_bytes {
        return Err(PortalErrorKind::IncompleteFrame.into());
    }
    let mut data = raw.data;
    if raw.format.force_opaque {
        force_opaque(&mut data, raw.width, raw.height, raw.stride);
    }
    let mut buffer = FrameBuffer {
        data: BytesMut::from(data.as_slice()),
        width: raw.width,
        height: raw.height,
        stride: raw.stride,
        format: raw.format.format,
    };
    if orientation == Orientation::Upright {
        buffer = to_native_orientation(buffer, output.transform)?;
    }
    Ok(Frame {
        buffer,
        output: OutputRef::from(output),
        scale: output.scale,
        transform: output.transform,
    })
}

/// The minimum byte count the geometry requires: full rows at the reported
/// stride, with the last row only as wide as the frame.
fn stride_floor(raw: &RawPwFrame) -> Result<usize, PortalScreenCastError> {
    let (width, height, stride) = (
        usize::try_from(raw.width)
            .map_err(|_| PortalErrorKind::Internal("frame width overflows usize"))?,
        usize::try_from(raw.height)
            .map_err(|_| PortalErrorKind::Internal("frame height overflows usize"))?,
        usize::try_from(raw.stride)
            .map_err(|_| PortalErrorKind::Internal("frame stride overflows usize"))?,
    );
    let row = width
        .checked_mul(4)
        .ok_or(PortalErrorKind::Internal("frame row bytes overflow usize"))?;
    if stride < row {
        return Err(
            PortalErrorKind::Internal("stream stride is narrower than the frame row").into(),
        );
    }
    height
        .checked_sub(1)
        .and_then(|rows| rows.checked_mul(stride))
        .and_then(|body| body.checked_add(row))
        .ok_or_else(|| PortalErrorKind::Internal("frame geometry overflows usize").into())
}

/// Forces the padding byte of every pixel opaque (`RGBx` -> `RGBA8888`).
fn force_opaque(data: &mut [u8], width: u32, height: u32, stride: u32) {
    let (Ok(width), Ok(height), Ok(stride)) = (
        usize::try_from(width),
        usize::try_from(height),
        usize::try_from(stride),
    ) else {
        return;
    };
    for row in 0..height {
        for column in 0..width {
            let Some(alpha) = data.get_mut(row * stride + column * 4 + 3) else {
                return;
            };
            *alpha = 255;
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use flowshot_capture::FrameFormat;
    use flowshot_core::geometry::{Logical, LogicalRect, PhysicalPx, PhysicalSize};

    use super::super::super::streams::MappedFormat;
    use super::*;

    fn output(transform: Transform, physical: (i32, i32)) -> OutputInfo {
        let buffer = transform.apply_to_size(flowshot_core::geometry::Size::new(
            PhysicalPx(physical.0),
            PhysicalPx(physical.1),
        ));
        OutputInfo::new(
            "TEST-1",
            "TEST-1",
            LogicalRect::new(
                Logical(0.0),
                Logical(0.0),
                Logical(f64::from(buffer.width.0)),
                Logical(f64::from(buffer.height.0)),
            ),
            PhysicalSize::new(PhysicalPx(physical.0), PhysicalPx(physical.1)),
            1.0,
            transform,
        )
        .unwrap()
    }

    fn raw(width: u32, height: u32, stride: u32, force_opaque: bool) -> RawPwFrame {
        let len = usize::try_from(stride).unwrap() * usize::try_from(height).unwrap();
        RawPwFrame {
            node_id: 42,
            width,
            height,
            stride,
            format: MappedFormat {
                format: FrameFormat::Rgba8888,
                force_opaque,
            },
            data: vec![7u8; len],
        }
    }

    #[test]
    fn native_frame_passes_through_with_metadata() {
        // Given a scale-1 normal output and a matching raw frame,
        let output = output(Transform::Normal, (4, 3));
        // When assembling,
        let frame = assemble_frame(&output, raw(4, 3, 16, false)).unwrap();
        // Then the frame carries the output's identity and metadata.
        assert_eq!((frame.buffer.width, frame.buffer.height), (4, 3));
        assert_eq!(frame.transform, Transform::Normal);
        assert_eq!(frame.output, OutputRef::Connector("TEST-1".to_owned()));
    }

    #[test]
    fn rgbx_padding_is_forced_opaque() {
        // Given an RGBx frame whose padding bytes are zero,
        let output = output(Transform::Normal, (2, 2));
        let mut frame_raw = raw(2, 2, 8, true);
        for pixel in 0..4 {
            frame_raw.data[pixel * 4 + 3] = 0;
        }
        // When assembling,
        let frame = assemble_frame(&output, frame_raw).unwrap();
        // Then every alpha byte is opaque while RGB survives.
        for y in 0..2 {
            for x in 0..2 {
                assert_eq!(frame.buffer.pixel(x, y), Some([7, 7, 7, 255]));
            }
        }
    }

    #[test]
    fn upright_rotated_frame_is_inverse_remapped_to_native() {
        // Given a Rot90 output (native 3x4) whose stream delivered the
        // UPRIGHT 4x3 buffer (mutter-style),
        let output = output(Transform::Rot90, (3, 4));
        let mut frame_raw = raw(4, 3, 16, false);
        for column in 0..4usize {
            frame_raw.data[(2 * 16) + column * 4] = 255; // bottom row red
        }
        // When assembling,
        let frame = assemble_frame(&output, frame_raw).unwrap();
        // Then the buffer is native 3x4 with the left column red.
        assert_eq!((frame.buffer.width, frame.buffer.height), (3, 4));
        assert_eq!(frame.transform, Transform::Rot90);
        assert_eq!(frame.buffer.pixel(0, 0), Some([255, 7, 7, 7]));
        assert_eq!(frame.buffer.pixel(0, 3), Some([255, 7, 7, 7]));
        assert_eq!(frame.buffer.pixel(1, 0), Some([7, 7, 7, 7]));
    }

    #[test]
    fn foreign_dimensions_are_a_typed_mismatch() {
        let output = output(Transform::Normal, (4, 3));
        let err = assemble_frame(&output, raw(8, 6, 32, false)).unwrap_err();
        assert!(matches!(
            err.kind(),
            PortalErrorKind::BufferSizeMismatch { .. }
        ));
    }

    #[test]
    fn short_data_is_incomplete() {
        let output = output(Transform::Normal, (4, 3));
        let mut frame_raw = raw(4, 3, 16, false);
        frame_raw.data.truncate(40);
        let err = assemble_frame(&output, frame_raw).unwrap_err();
        assert!(matches!(err.kind(), PortalErrorKind::IncompleteFrame));
    }

    #[test]
    fn stride_narrower_than_the_row_is_internal() {
        let output = output(Transform::Normal, (4, 3));
        let err = assemble_frame(&output, raw(4, 3, 8, false)).unwrap_err();
        assert!(matches!(err.kind(), PortalErrorKind::Internal(_)));
    }
}
