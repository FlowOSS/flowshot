//! Portal composite geometry: pixel-space detection and per-output cropping.
//!
//! The Screenshot portal returns ONE full-desktop image with no geometry
//! metadata. Its pixel space is implementation-defined (`XDPH` composites
//! with `grim` and delivers PHYSICAL pixels; other frontends may deliver
//! logical), so it is DETECTED at runtime by comparing the decoded image
//! dimensions against the layout's physical and logical bounding boxes.
//! Physical space crops exactly; logical space with any
//! output at scale != 1 is a typed v1 limitation - honoring the
//! physical-first rule (never rescale) is impossible from a downsampled
//! composite.
//!
//! The physical composite model matches `grim`: each output contributes its
//! post-transform physical buffer at `round(logical_origin * own_scale)`,
//! and the composite spans the union bounding box. Cropped per-output
//! buffers are UPRIGHT (post-transform), so they are inverse-remapped into
//! the shared [`Frame`] contract (native pixels + transform metadata), the
//! same normalization the ICC backend performs.

use bytes::BytesMut;
use flowshot_capture::{Frame, FrameBuffer, FrameFormat, OutputRef};
use flowshot_core::geometry::{OutputInfo, Transform};

use super::error::PortalErrorKind;
use crate::stitch::round_to_i32;

/// One output's crop rectangle inside the physical composite, in composite
/// pixel space (origin at the bounding-box top-left).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CompositeCrop {
    /// Horizontal offset of the output's upright buffer in the composite.
    pub x: i32,
    /// Vertical offset of the output's upright buffer in the composite.
    pub y: i32,
    /// Post-transform physical width.
    pub width: i32,
    /// Post-transform physical height.
    pub height: i32,
}

/// The physical-pixel layout of a portal composite: its bounding-box
/// dimensions plus every output's crop rectangle, parallel to the output
/// list it was built from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompositeLayout {
    /// Composite width in physical pixels.
    pub width: u32,
    /// Composite height in physical pixels.
    pub height: u32,
    /// Per-output crop rectangles (`crops[i]` belongs to `outputs[i]`).
    pub crops: Vec<CompositeCrop>,
}

/// Computes the physical composite layout for `outputs` (the `grim` model).
///
/// # Errors
///
/// [`PortalErrorKind::NoOutputs`] for an empty layout and
/// [`PortalErrorKind::Internal`] on dimension overflow.
pub(crate) fn composite_layout(outputs: &[OutputInfo]) -> Result<CompositeLayout, PortalErrorKind> {
    if outputs.is_empty() {
        return Err(PortalErrorKind::NoOutputs);
    }
    let mut rects = Vec::with_capacity(outputs.len());
    for output in outputs {
        let buffer = output.buffer_size();
        rects.push((
            round_to_i32(output.logical_rect.x.0 * output.scale),
            round_to_i32(output.logical_rect.y.0 * output.scale),
            buffer.width.0,
            buffer.height.0,
        ));
    }
    let origin_x = rects.iter().map(|rect| rect.0).min();
    let origin_y = rects.iter().map(|rect| rect.1).min();
    let (Some(origin_x), Some(origin_y)) = (origin_x, origin_y) else {
        return Err(PortalErrorKind::Internal("empty composite layout"));
    };
    let edge_x = rects.iter().map(|rect| rect.0.saturating_add(rect.2)).max();
    let edge_y = rects.iter().map(|rect| rect.1.saturating_add(rect.3)).max();
    let (Some(edge_x), Some(edge_y)) = (edge_x, edge_y) else {
        return Err(PortalErrorKind::Internal("empty composite layout"));
    };
    let width = u32::try_from(edge_x.saturating_sub(origin_x).max(0))
        .map_err(|_| PortalErrorKind::Internal("composite width overflows u32"))?;
    let height = u32::try_from(edge_y.saturating_sub(origin_y).max(0))
        .map_err(|_| PortalErrorKind::Internal("composite height overflows u32"))?;
    let crops = rects
        .into_iter()
        .map(|(x, y, width, height)| CompositeCrop {
            x: x - origin_x,
            y: y - origin_y,
            width,
            height,
        })
        .collect();
    Ok(CompositeLayout {
        width,
        height,
        crops,
    })
}

/// Detects the composite's pixel space from its dimensions (DETECT AT
/// RUNTIME, never assume).
///
/// # Errors
///
/// [`PortalErrorKind::LogicalSpaceUnsupported`] when the image matches the
/// LOGICAL bounding box but not the physical one (a `HiDPI` composite this
/// backend refuses to resample), and [`PortalErrorKind::UnknownPixelSpace`]
/// when it matches neither.
pub(crate) fn verify_pixel_space(
    image: (u32, u32),
    outputs: &[OutputInfo],
    layout: &CompositeLayout,
) -> Result<(), PortalErrorKind> {
    if image == (layout.width, layout.height) {
        return Ok(());
    }
    let logical = logical_bounds_size(outputs);
    if (i64::from(image.0), i64::from(image.1)) == logical {
        return Err(PortalErrorKind::LogicalSpaceUnsupported { image });
    }
    let clamp_dim = |value: i64| {
        i32::try_from(value.clamp(i64::from(i32::MIN), i64::from(i32::MAX))).unwrap_or(i32::MAX)
    };
    Err(PortalErrorKind::UnknownPixelSpace {
        image,
        physical: (
            clamp_dim(layout.width.into()),
            clamp_dim(layout.height.into()),
        ),
        logical: (clamp_dim(logical.0), clamp_dim(logical.1)),
    })
}

/// The logical bounding-box size of the layout, for pixel-space detection.
fn logical_bounds_size(outputs: &[OutputInfo]) -> (i64, i64) {
    let layout = flowshot_core::geometry::OutputLayout::new(outputs.to_vec());
    let Some(bounds) = layout.union_bounds() else {
        return (0, 0);
    };
    (
        round_to_i32(bounds.width.0).into(),
        round_to_i32(bounds.height.0).into(),
    )
}

/// Crops the decoded RGBA composite into per-output [`Frame`]s (native
/// orientation + transform metadata, the shared contract).
///
/// # Errors
///
/// [`PortalErrorKind::Internal`] when a crop falls outside the image or an
/// offset overflows, and [`PortalErrorKind::Geometry`] when an inverse
/// remap rejects a buffer.
pub(crate) fn crop_frames(
    rgba: &[u8],
    image: (u32, u32),
    outputs: &[OutputInfo],
    layout: &CompositeLayout,
) -> Result<Vec<Frame>, PortalErrorKind> {
    let (width, height) = image;
    let row_bytes = usize::try_from(width)
        .ok()
        .and_then(|w| w.checked_mul(4))
        .ok_or(PortalErrorKind::Internal("composite width overflows usize"))?;
    let expected = row_bytes
        .checked_mul(usize::try_from(height).unwrap_or(usize::MAX))
        .ok_or(PortalErrorKind::Internal("composite size overflows usize"))?;
    if rgba.len() < expected {
        return Err(PortalErrorKind::IncompleteFrame);
    }
    let mut frames = Vec::with_capacity(outputs.len());
    for (output, crop) in outputs.iter().zip(&layout.crops) {
        let upright = crop_upright(rgba, row_bytes, crop)?;
        let buffer = FrameBuffer {
            data: BytesMut::from(upright.as_slice()),
            width: u32::try_from(crop.width)
                .map_err(|_| PortalErrorKind::Internal("crop width overflows u32"))?,
            height: u32::try_from(crop.height)
                .map_err(|_| PortalErrorKind::Internal("crop height overflows u32"))?,
            stride: u32::try_from(crop.width)
                .map_err(|_| PortalErrorKind::Internal("crop stride overflows u32"))?
                .checked_mul(4)
                .ok_or(PortalErrorKind::Internal("crop stride overflows u32"))?,
            format: FrameFormat::Rgba8888,
        };
        let buffer = to_native_orientation(buffer, output.transform)?;
        frames.push(Frame {
            buffer,
            output: OutputRef::from(output),
            scale: output.scale,
            transform: output.transform,
        });
    }
    Ok(frames)
}

/// Copies one output's upright pixels out of the composite.
fn crop_upright(
    rgba: &[u8],
    row_bytes: usize,
    crop: &CompositeCrop,
) -> Result<Vec<u8>, PortalErrorKind> {
    let (x, y, width, height) = (
        usize::try_from(crop.x)
            .map_err(|_| PortalErrorKind::Internal("composite crop is negative"))?,
        usize::try_from(crop.y)
            .map_err(|_| PortalErrorKind::Internal("composite crop is negative"))?,
        usize::try_from(crop.width)
            .map_err(|_| PortalErrorKind::Internal("composite crop is negative"))?,
        usize::try_from(crop.height)
            .map_err(|_| PortalErrorKind::Internal("composite crop is negative"))?,
    );
    let mut data = Vec::with_capacity(
        width
            .checked_mul(height)
            .and_then(|px| px.checked_mul(4))
            .ok_or(PortalErrorKind::Internal("composite crop overflows usize"))?,
    );
    let overflow = || PortalErrorKind::Internal("composite crop overflows the image");
    let x_bytes = x.checked_mul(4).ok_or_else(overflow)?;
    let row_width = width.checked_mul(4).ok_or_else(overflow)?;
    for row in y..y.checked_add(height).ok_or_else(overflow)? {
        let start = row
            .checked_mul(row_bytes)
            .and_then(|offset| offset.checked_add(x_bytes))
            .ok_or_else(overflow)?;
        let end = start.checked_add(row_width).ok_or_else(overflow)?;
        let Some(row_pixels) = rgba.get(start..end) else {
            return Err(PortalErrorKind::Internal(
                "portal composite does not cover an enumerated output",
            ));
        };
        data.extend_from_slice(row_pixels);
    }
    Ok(data)
}

/// Inverse-remaps an upright (post-transform) buffer back to the output's
/// native orientation, per the shared [`Frame`] contract (the same
/// normalization the ICC backend applies; `wlr-screencopy` and the portal
/// `ScreenCast` path may already deliver native pixels and skip this).
///
/// # Errors
///
/// [`PortalErrorKind::Internal`] on dimension conversion failures and
/// [`PortalErrorKind::Geometry`] when the remap rejects the buffers.
pub(crate) fn to_native_orientation(
    buffer: FrameBuffer,
    transform: Transform,
) -> Result<FrameBuffer, PortalErrorKind> {
    if transform == Transform::Normal {
        return Ok(buffer);
    }
    let inverse = transform.inverse();
    let (Ok(width), Ok(height)) = (
        usize::try_from(buffer.width),
        usize::try_from(buffer.height),
    ) else {
        return Err(PortalErrorKind::Internal(
            "buffer dimensions overflow usize",
        ));
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
        .ok_or(PortalErrorKind::Internal("native stride overflows u32"))?;
    Ok(FrameBuffer {
        data: BytesMut::from(data.as_slice()),
        width,
        height,
        stride,
        format: buffer.format,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use flowshot_core::geometry::{Logical, LogicalRect, PhysicalPx, PhysicalSize};

    use super::*;

    fn output(
        connector: &str,
        logical: (f64, f64, f64, f64),
        physical: (i32, i32),
        scale: f64,
        transform: Transform,
    ) -> OutputInfo {
        OutputInfo::new(
            connector,
            connector,
            LogicalRect::new(
                Logical(logical.0),
                Logical(logical.1),
                Logical(logical.2),
                Logical(logical.3),
            ),
            PhysicalSize::new(PhysicalPx(physical.0), PhysicalPx(physical.1)),
            scale,
            transform,
        )
        .unwrap()
    }

    /// The live QA layout: HDMI-A-1 1920x1080 at (0,0) + DP-3 2560x1440 at
    /// (1920,0), both scale 1 -> composite 4480x1440.
    fn live_layout() -> Vec<OutputInfo> {
        vec![
            output(
                "HDMI-A-1",
                (0.0, 0.0, 1920.0, 1080.0),
                (1920, 1080),
                1.0,
                Transform::Normal,
            ),
            output(
                "DP-3",
                (1920.0, 0.0, 2560.0, 1440.0),
                (2560, 1440),
                1.0,
                Transform::Normal,
            ),
        ]
    }

    fn solid_rgba(width: usize, height: usize, pixel: [u8; 4]) -> Vec<u8> {
        let mut data = Vec::with_capacity(width * height * 4);
        for _ in 0..width * height {
            data.extend_from_slice(&pixel);
        }
        data
    }

    #[test]
    fn live_layout_composite_is_4480x1440_with_side_by_side_crops() {
        let outputs = live_layout();
        let layout = composite_layout(&outputs).unwrap();
        assert_eq!((layout.width, layout.height), (4480, 1440));
        assert_eq!(
            layout.crops,
            vec![
                CompositeCrop {
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 1080
                },
                CompositeCrop {
                    x: 1920,
                    y: 0,
                    width: 2560,
                    height: 1440
                },
            ]
        );
    }

    #[test]
    fn scale_two_output_lands_at_rounded_physical_origin() {
        // The grim placement model: each output's physical origin is its
        // logical origin times its OWN scale. A scale-2 output (logical
        // 960x540, physical 1920x1080) at (0,0) next to a scale-1 output at
        // logical (1920,0) yields a 2720x1080 composite.
        let outputs = vec![
            output(
                "HI",
                (0.0, 0.0, 960.0, 540.0),
                (1920, 1080),
                2.0,
                Transform::Normal,
            ),
            output(
                "LO",
                (1920.0, 0.0, 800.0, 600.0),
                (800, 600),
                1.0,
                Transform::Normal,
            ),
        ];
        let layout = composite_layout(&outputs).unwrap();
        assert_eq!(layout.crops[0].x, 0);
        assert_eq!(layout.crops[1].x, 1920);
        assert_eq!((layout.width, layout.height), (2720, 1080));
    }

    #[test]
    fn physical_composite_passes_detection() {
        let outputs = live_layout();
        let layout = composite_layout(&outputs).unwrap();
        assert!(verify_pixel_space((4480, 1440), &outputs, &layout).is_ok());
    }

    #[test]
    fn logical_composite_on_hidpi_layout_is_the_typed_limitation() {
        let outputs = vec![output(
            "HI",
            (0.0, 0.0, 960.0, 540.0),
            (1920, 1080),
            2.0,
            Transform::Normal,
        )];
        let layout = composite_layout(&outputs).unwrap();
        let err = verify_pixel_space((960, 540), &outputs, &layout).unwrap_err();
        assert!(matches!(
            err,
            PortalErrorKind::LogicalSpaceUnsupported { image: (960, 540) }
        ));
    }

    #[test]
    fn unmatched_dimensions_report_both_candidate_spaces() {
        let outputs = live_layout();
        let layout = composite_layout(&outputs).unwrap();
        let err = verify_pixel_space((1234, 567), &outputs, &layout).unwrap_err();
        match err {
            PortalErrorKind::UnknownPixelSpace {
                image,
                physical,
                logical,
            } => {
                assert_eq!(image, (1234, 567));
                assert_eq!(physical, (4480, 1440));
                assert_eq!(logical, (4480, 1440));
            }
            other => panic!("expected UnknownPixelSpace, got {other:?}"),
        }
    }

    #[test]
    fn crops_split_the_composite_per_output() {
        // Given a 8x3 composite: red left 4 columns, green right 4 columns,
        // and two 4x3 scale-1 outputs side by side,
        let outputs = vec![
            output("A", (0.0, 0.0, 4.0, 3.0), (4, 3), 1.0, Transform::Normal),
            output("B", (4.0, 0.0, 4.0, 3.0), (4, 3), 1.0, Transform::Normal),
        ];
        let layout = composite_layout(&outputs).unwrap();
        let mut rgba = solid_rgba(4, 3, [255, 0, 0, 255]);
        rgba.extend_from_slice(&solid_rgba(4, 3, [0, 255, 0, 255]));
        // When cropping,
        let frames = crop_frames(&rgba, (8, 3), &outputs, &layout).unwrap();
        // Then each frame carries its output's color, connector, and metadata.
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].buffer.pixel(0, 0), Some([255, 0, 0, 255]));
        assert_eq!(frames[1].buffer.pixel(3, 2), Some([0, 255, 0, 255]));
        assert_eq!(frames[0].output, OutputRef::Connector("A".to_owned()),);
        assert_eq!(frames[1].transform, Transform::Normal);
    }

    #[test]
    fn rotated_output_crop_is_inverse_remapped_to_native() {
        // Given a Rot90 output: native 3x4, upright 4x3 in the composite.
        // The upright crop has its BOTTOM row red (the stitch tests' forward
        // fixture); the inverse remap (Rot270) must place it as the native
        // buffer's LEFT column: upright (x, 2) -> native (0, x).
        let outputs = vec![output(
            "R",
            (0.0, 0.0, 4.0, 3.0),
            (3, 4),
            1.0,
            Transform::Rot90,
        )];
        let layout = composite_layout(&outputs).unwrap();
        let mut rgba = Vec::new();
        for y in 0..3usize {
            for _x in 0..4usize {
                rgba.extend_from_slice(if y == 2 {
                    &[255, 0, 0, 255]
                } else {
                    &[0, 255, 0, 255]
                });
            }
        }
        // When cropping,
        let frames = crop_frames(&rgba, (4, 3), &outputs, &layout).unwrap();
        // Then the frame is native 3x4 with the transform as metadata and
        // the red row back on the left column.
        assert_eq!((frames[0].buffer.width, frames[0].buffer.height), (3, 4));
        assert_eq!(frames[0].transform, Transform::Rot90);
        assert_eq!(frames[0].buffer.pixel(0, 0), Some([255, 0, 0, 255]));
        assert_eq!(frames[0].buffer.pixel(0, 3), Some([255, 0, 0, 255]));
        assert_eq!(frames[0].buffer.pixel(1, 0), Some([0, 255, 0, 255]));
        assert_eq!(frames[0].buffer.pixel(2, 3), Some([0, 255, 0, 255]));
    }

    #[test]
    fn short_composite_data_is_incomplete_not_a_panic() {
        let outputs = live_layout();
        let layout = composite_layout(&outputs).unwrap();
        let err = crop_frames(&[0u8; 16], (4480, 1440), &outputs, &layout).unwrap_err();
        assert!(matches!(err, PortalErrorKind::IncompleteFrame));
    }

    #[test]
    fn empty_layout_is_a_typed_error() {
        assert!(matches!(
            composite_layout(&[]).unwrap_err(),
            PortalErrorKind::NoOutputs
        ));
    }
}
