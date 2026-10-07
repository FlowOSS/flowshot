//! Region stitching: per-output [`Frame`]s placed into a single composite
//! through the physical-first [`OutputLayout`] algebra of `flowshot-core`.
//!
//! The stitched frame follows the shared region-capture contract (identical
//! to [`MockBackend`](crate::MockBackend)): destination scale 1.0 - one
//! pixel per logical unit -
//! [`OutputRef::Composite`], and [`Transform::Normal`]. Each output
//! contributes its physical crop (computed with THAT output's scale, never
//! an averaged factor), resampled nearest-neighbour into its logical
//! destination rect; rotated outputs are remapped to layout orientation
//! first. Sequential per-output captures mean fast-moving content can skew
//! between outputs (`grim`-equivalent, documented in the runner).
//!
//! This module is the shared, platform-free stitch algebra BOTH platform
//! crates consume (each re-exports it as its own `stitch`): the executable
//! form of the shared region-capture contract. `kind` tags every error
//! with the calling backend.

use bytes::BytesMut;
use flowshot_core::geometry::{LogicalRect, OutputInfo, OutputLayout, PhysicalRect, Transform};

use crate::{BackendKind, CaptureError, Frame, FrameBuffer, FrameFormat, OutputRef};

/// The outputs of one capture run paired with their frames.
///
/// Produced by a capture pass and consumed by [`CapturedOutputs::stitch`];
/// `frames[i]` belongs to `outputs[i]`.
#[derive(Debug, Clone, PartialEq)]
pub struct CapturedOutputs {
    /// The captured outputs, in capture order.
    pub outputs: Vec<OutputInfo>,
    /// One frame per output, native pre-transform orientation with the
    /// output's transform as metadata (the shared [`Frame`] contract).
    pub frames: Vec<Frame>,
}

impl CapturedOutputs {
    /// Stitches the captured frames covering `region` (global logical space)
    /// into one composite frame at scale 1.0.
    ///
    /// `kind` tags the errors with the backend that produced the frames.
    ///
    /// # Errors
    ///
    /// [`CaptureError::RegionOutsideLayout`] when `region` does not
    /// intersect the layout or rounds to zero pixels, and
    /// [`CaptureError::Backend`] when an output's frame is missing, a crop
    /// falls outside its frame, or a remap rejects a buffer.
    pub fn stitch(&self, kind: BackendKind, region: LogicalRect) -> Result<Frame, CaptureError> {
        let layout = OutputLayout::new(self.outputs.clone());
        let clamped = layout
            .clamp_region_to_layout(region)
            .ok_or(CaptureError::RegionOutsideLayout { region })?;
        let (width, height) =
            stitched_size(&clamped).ok_or(CaptureError::RegionOutsideLayout { region })?;
        let destination_width = usize_dim(kind, width)?;
        let destination_height = usize_dim(kind, height)?;
        let pixel_count = destination_width
            .checked_mul(destination_height)
            .ok_or_else(|| internal_error(kind, "region dimensions overflow usize"))?;
        let mut data = vec![
            0u8;
            pixel_count.checked_mul(4).ok_or_else(|| internal_error(
                kind,
                "region dimensions overflow usize"
            ))?
        ];

        for crop in layout.crop_rects(clamped) {
            let frame = self
                .frame_for(&crop.output.connector)
                .ok_or_else(|| internal_error(kind, "capture produced no frame for an output"))?;
            let upright = oriented(kind, &frame.buffer, frame.transform)?;
            blit_crop(
                kind,
                &upright,
                crop.physical,
                DestinationRect::of(&crop.logical, &clamped),
                &mut data,
                destination_width,
            )?;
        }

        Ok(Frame {
            buffer: FrameBuffer {
                data: BytesMut::from(data.as_slice()),
                width,
                height,
                stride: width
                    .checked_mul(4)
                    .ok_or_else(|| internal_error(kind, "region stride overflows u32"))?,
                format: FrameFormat::Rgba8888,
            },
            output: OutputRef::Composite,
            scale: 1.0,
            transform: Transform::Normal,
        })
    }

    fn frame_for(&self, connector: &str) -> Option<&Frame> {
        self.frames
            .iter()
            .find(|frame| frame.output == OutputRef::Connector(connector.to_owned()))
    }
}

/// Converts one pixel of any v1 frame format to byte order R, G, B, A.
///
/// Shared by the stitcher and the QA harnesses (PNG encoding wants RGBA).
#[must_use]
pub fn to_rgba(format: FrameFormat, pixel: [u8; 4]) -> [u8; 4] {
    match format {
        // Little-endian 0x00RRGGBB: bytes B, G, R, X - opaque.
        FrameFormat::Xrgb8888 => [pixel[2], pixel[1], pixel[0], 255],
        // Little-endian 0xAARRGGBB: bytes B, G, R, A.
        FrameFormat::Argb8888 => [pixel[2], pixel[1], pixel[0], pixel[3]],
        FrameFormat::Rgba8888 => pixel,
    }
}

/// The stitched destination size for a clamped region at scale 1.0, or
/// `None` when it rounds to zero pixels.
fn stitched_size(clamped: &LogicalRect) -> Option<(u32, u32)> {
    let origin_x = round_to_i32(clamped.x.0);
    let origin_y = round_to_i32(clamped.y.0);
    let width = round_to_i32(clamped.right().0).saturating_sub(origin_x);
    let height = round_to_i32(clamped.bottom().0).saturating_sub(origin_y);
    Some((
        u32::try_from(width).ok().filter(|value| *value > 0)?,
        u32::try_from(height).ok().filter(|value| *value > 0)?,
    ))
}

/// Rounds a logical edge to an integer pixel edge (half away from zero),
/// saturating at the `i32` bounds; non-finite input rounds to `0`.
///
/// Shared with the Wayland portal composite geometry
/// (`flowshot-capture-wayland`'s `portal` module), which rounds logical
/// output origins into physical composite space the same way.
#[must_use]
pub fn round_to_i32(value: f64) -> i32 {
    if !value.is_finite() {
        return 0;
    }
    let clamped = value
        .round()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX));
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the value is clamped to the representable i32 range above"
    )]
    let edge = clamped as i32;
    edge
}

/// One output's share of a stitched region frame: its destination rectangle
/// in the stitched buffer, at the stitched frame's scale of 1.0.
#[derive(Debug, Clone, Copy)]
struct DestinationRect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl DestinationRect {
    /// Derives the destination rect from the output's logical intersection
    /// relative to the clamped region origin.
    fn of(logical: &LogicalRect, clamped: &LogicalRect) -> Self {
        let x = round_to_i32((logical.x - clamped.x).0).max(0);
        let y = round_to_i32((logical.y - clamped.y).0).max(0);
        let right = round_to_i32((logical.right() - clamped.x).0).max(x);
        let bottom = round_to_i32((logical.bottom() - clamped.y).0).max(y);
        Self {
            x,
            y,
            width: right - x,
            height: bottom - y,
        }
    }
}

/// Remaps a native-orientation buffer into post-transform (layout)
/// orientation, converting nothing: the format travels with the buffer.
/// Normal orientation borrows instead of copying.
///
/// # Errors
///
/// [`CaptureError::Backend`] when the remap rejects the buffers.
fn oriented(
    kind: BackendKind,
    buffer: &FrameBuffer,
    transform: Transform,
) -> Result<std::borrow::Cow<'_, FrameBuffer>, CaptureError> {
    if transform == Transform::Normal {
        return Ok(std::borrow::Cow::Borrowed(buffer));
    }
    let width = usize_dim(kind, buffer.width)?;
    let height = usize_dim(kind, buffer.height)?;
    let mut data = vec![0u8; buffer.data.len()];
    transform
        .remap_buffer(&buffer.data, &mut data, width, height, 4)
        .map_err(|error| CaptureError::Backend {
            backend: kind,
            source: error.into(),
        })?;
    let (width, height) = if transform.swaps_dimensions() {
        (buffer.height, buffer.width)
    } else {
        (buffer.width, buffer.height)
    };
    Ok(std::borrow::Cow::Owned(FrameBuffer {
        data: BytesMut::from(data.as_slice()),
        width,
        height,
        stride: width
            .checked_mul(4)
            .ok_or_else(|| internal_error(kind, "oriented stride overflows u32"))?,
        format: buffer.format,
    }))
}

/// Nearest-neighbour blits one output's physical crop into the stitched
/// `RGBA8888` destination, converting each sampled pixel.
///
/// # Errors
///
/// [`CaptureError::Backend`] when a crop coordinate is negative, a sample
/// falls outside the source frame, or a conversion overflows `usize`.
fn blit_crop(
    kind: BackendKind,
    source: &FrameBuffer,
    physical: PhysicalRect,
    destination: DestinationRect,
    data: &mut [u8],
    destination_width: usize,
) -> Result<(), CaptureError> {
    let edge =
        |value: i32, what: &str| usize::try_from(value).map_err(|_| internal_error(kind, what));
    let source_x = edge(physical.x.0, "crop rect is negative")?;
    let source_y = edge(physical.y.0, "crop rect is negative")?;
    let crop_width = edge(physical.width.0, "crop rect is negative")?;
    let crop_height = edge(physical.height.0, "crop rect is negative")?;
    let dest_x = edge(destination.x, "destination rect overflows usize")?;
    let dest_y = edge(destination.y, "destination rect overflows usize")?;
    let dest_width = edge(destination.width, "destination rect overflows usize")?;
    let dest_height = edge(destination.height, "destination rect overflows usize")?;
    if crop_width == 0 || crop_height == 0 || dest_width == 0 || dest_height == 0 {
        return Ok(());
    }

    for row in 0..dest_height {
        let sample_y = source_y + row * crop_height / dest_height;
        for column in 0..dest_width {
            let sample_x = source_x + column * crop_width / dest_width;
            let pixel = source.pixel(
                u32::try_from(sample_x)
                    .map_err(|_| internal_error(kind, "crop sample overflows u32"))?,
                u32::try_from(sample_y)
                    .map_err(|_| internal_error(kind, "crop sample overflows u32"))?,
            );
            let Some(pixel) = pixel else {
                return Err(internal_error(kind, "crop sample fell outside the frame"));
            };
            let rgba = to_rgba(source.format, pixel);
            let offset = ((dest_y + row) * destination_width + dest_x + column) * 4;
            let Some(slot) = data.get_mut(offset..offset + 4) else {
                return Err(internal_error(
                    kind,
                    "destination offset overflows the stitch",
                ));
            };
            slot.copy_from_slice(&rgba);
        }
    }
    Ok(())
}

fn usize_dim(kind: BackendKind, value: u32) -> Result<usize, CaptureError> {
    usize::try_from(value).map_err(|_| internal_error(kind, "dimension overflows usize"))
}

fn internal_error(kind: BackendKind, message: &str) -> CaptureError {
    CaptureError::Backend {
        backend: kind,
        source: message.to_owned().into(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    // Fixture scales and sizes are exact integral literals (geometry-notepad
    // convention), so strict comparison is the intended assertion.
    #![allow(clippy::float_cmp)]

    use flowshot_core::geometry::{Logical, PhysicalPx, PhysicalSize};

    use super::*;

    const RED: [u8; 4] = [255, 0, 0, 255];
    const GREEN: [u8; 4] = [0, 255, 0, 255];

    fn output(
        connector: &str,
        logical: LogicalRect,
        physical: (i32, i32),
        scale: f64,
        transform: Transform,
    ) -> OutputInfo {
        OutputInfo::new(
            connector,
            connector,
            logical,
            PhysicalSize::new(PhysicalPx(physical.0), PhysicalPx(physical.1)),
            scale,
            transform,
        )
        .unwrap()
    }

    fn solid_frame(
        connector: &str,
        width: u32,
        height: u32,
        color: [u8; 4],
        format: FrameFormat,
        transform: Transform,
        scale: f64,
    ) -> Frame {
        let pixel = match format {
            FrameFormat::Rgba8888 => color,
            // Store the RGBA color in the format's own byte order so
            // to_rgba() round-trips it.
            FrameFormat::Xrgb8888 | FrameFormat::Argb8888 => {
                [color[2], color[1], color[0], color[3]]
            }
        };
        let pixels = usize::try_from(width * height).unwrap();
        let mut data = Vec::with_capacity(pixels * 4);
        for _ in 0..pixels {
            data.extend_from_slice(&pixel);
        }
        Frame {
            buffer: FrameBuffer {
                data: BytesMut::from(data.as_slice()),
                width,
                height,
                stride: width * 4,
                format,
            },
            output: OutputRef::Connector(connector.to_owned()),
            scale,
            transform,
        }
    }

    fn region(x: f64, y: f64, width: f64, height: f64) -> LogicalRect {
        LogicalRect::new(Logical(x), Logical(y), Logical(width), Logical(height))
    }

    #[test]
    fn to_rgba_converts_every_v1_format() {
        // Bytes B=1, G=2, R=3, X/A=4 in memory.
        let bytes = [1, 2, 3, 4];
        assert_eq!(
            to_rgba(FrameFormat::Xrgb8888, bytes),
            [3, 2, 1, 255],
            "XRGB is opaque"
        );
        assert_eq!(to_rgba(FrameFormat::Argb8888, bytes), [3, 2, 1, 4]);
        assert_eq!(to_rgba(FrameFormat::Rgba8888, bytes), bytes);
    }

    #[test]
    fn two_abutting_outputs_stitch_side_by_side() {
        // Given two scale-1 outputs: red 4x3 at (0,0), green 4x3 at (4,0).
        let outputs = vec![
            output(
                "A",
                region(0.0, 0.0, 4.0, 3.0),
                (4, 3),
                1.0,
                Transform::Normal,
            ),
            output(
                "B",
                region(4.0, 0.0, 4.0, 3.0),
                (4, 3),
                1.0,
                Transform::Normal,
            ),
        ];
        let captured = CapturedOutputs {
            frames: vec![
                solid_frame(
                    "A",
                    4,
                    3,
                    RED,
                    FrameFormat::Xrgb8888,
                    Transform::Normal,
                    1.0,
                ),
                solid_frame(
                    "B",
                    4,
                    3,
                    GREEN,
                    FrameFormat::Argb8888,
                    Transform::Normal,
                    1.0,
                ),
            ],
            outputs: outputs.clone(),
        };
        // When stitching the full 8x3 span,
        let frame = captured
            .stitch(BackendKind::ExtImageCopyCapture, region(0.0, 0.0, 8.0, 3.0))
            .unwrap();
        // Then the composite is 8x3 RGBA with each output's color on its side.
        assert_eq!((frame.buffer.width, frame.buffer.height), (8, 3));
        assert_eq!(frame.output, OutputRef::Composite);
        assert_eq!(frame.scale, 1.0);
        assert_eq!(frame.transform, Transform::Normal);
        assert_eq!(frame.buffer.pixel(3, 1), Some(RED));
        assert_eq!(frame.buffer.pixel(4, 1), Some(GREEN));
    }

    #[test]
    fn scale_two_output_downsamples_to_logical_destination() {
        // Given one scale-2 output: logical 4x3, physical 8x6.
        let outputs = vec![output(
            "H",
            region(0.0, 0.0, 4.0, 3.0),
            (8, 6),
            2.0,
            Transform::Normal,
        )];
        let captured = CapturedOutputs {
            frames: vec![solid_frame(
                "H",
                8,
                6,
                GREEN,
                FrameFormat::Xrgb8888,
                Transform::Normal,
                2.0,
            )],
            outputs,
        };
        // When stitching its full logical extent,
        let frame = captured
            .stitch(BackendKind::ExtImageCopyCapture, region(0.0, 0.0, 4.0, 3.0))
            .unwrap();
        // Then the destination is the LOGICAL size (scale-1.0 contract).
        assert_eq!((frame.buffer.width, frame.buffer.height), (4, 3));
        assert_eq!(frame.buffer.pixel(2, 2), Some(GREEN));
    }

    #[test]
    fn rotated_output_is_remapped_into_layout_orientation() {
        // Given a Rot90 output: native 3x4 buffer, post-transform 4x3.
        let outputs = vec![output(
            "R",
            region(0.0, 0.0, 4.0, 3.0),
            (3, 4),
            1.0,
            Transform::Rot90,
        )];
        // Native buffer: left column (x=0) red, rest green. Rot90's
        // map_point sends native (0, y) to upright (y, last_x) - the left
        // column becomes the BOTTOM row of the upright image.
        let mut data = Vec::new();
        for _y in 0..4usize {
            for x in 0..3usize {
                let color = if x == 0 { RED } else { GREEN };
                data.extend_from_slice(&[color[2], color[1], color[0], 255]);
            }
        }
        let frame = Frame {
            buffer: FrameBuffer {
                data: BytesMut::from(data.as_slice()),
                width: 3,
                height: 4,
                stride: 12,
                format: FrameFormat::Xrgb8888,
            },
            output: OutputRef::Connector("R".to_owned()),
            scale: 1.0,
            transform: Transform::Rot90,
        };
        let captured = CapturedOutputs {
            frames: vec![frame],
            outputs,
        };
        // When stitching the full logical extent,
        let stitched = captured
            .stitch(BackendKind::ExtImageCopyCapture, region(0.0, 0.0, 4.0, 3.0))
            .unwrap();
        // Then the upright image shows the red column as the bottom row.
        assert_eq!((stitched.buffer.width, stitched.buffer.height), (4, 3));
        assert_eq!(stitched.buffer.pixel(0, 2), Some(RED));
        assert_eq!(stitched.buffer.pixel(3, 2), Some(RED));
        assert_eq!(stitched.buffer.pixel(0, 0), Some(GREEN));
        assert_eq!(stitched.buffer.pixel(0, 1), Some(GREEN));
    }

    #[test]
    fn region_outside_the_layout_is_a_typed_error() {
        let outputs = vec![output(
            "A",
            region(0.0, 0.0, 4.0, 3.0),
            (4, 3),
            1.0,
            Transform::Normal,
        )];
        let captured = CapturedOutputs {
            frames: vec![solid_frame(
                "A",
                4,
                3,
                RED,
                FrameFormat::Xrgb8888,
                Transform::Normal,
                1.0,
            )],
            outputs,
        };
        let err = captured
            .stitch(
                BackendKind::ExtImageCopyCapture,
                region(100.0, 100.0, 4.0, 4.0),
            )
            .unwrap_err();
        assert!(matches!(err, CaptureError::RegionOutsideLayout { .. }));
    }

    #[test]
    fn missing_frame_for_a_cropped_output_is_a_typed_error() {
        let outputs = vec![
            output(
                "A",
                region(0.0, 0.0, 4.0, 3.0),
                (4, 3),
                1.0,
                Transform::Normal,
            ),
            output(
                "B",
                region(4.0, 0.0, 4.0, 3.0),
                (4, 3),
                1.0,
                Transform::Normal,
            ),
        ];
        let captured = CapturedOutputs {
            frames: vec![solid_frame(
                "A",
                4,
                3,
                RED,
                FrameFormat::Xrgb8888,
                Transform::Normal,
                1.0,
            )],
            outputs,
        };
        let err = captured
            .stitch(BackendKind::ExtImageCopyCapture, region(0.0, 0.0, 8.0, 3.0))
            .unwrap_err();
        assert!(matches!(err, CaptureError::Backend { .. }));
    }

    #[test]
    fn partial_region_crops_each_output_physically() {
        // Given red 8x6 at scale 1 and green 8x6 at scale 1 side by side,
        let outputs = vec![
            output(
                "A",
                region(0.0, 0.0, 8.0, 6.0),
                (8, 6),
                1.0,
                Transform::Normal,
            ),
            output(
                "B",
                region(8.0, 0.0, 8.0, 6.0),
                (8, 6),
                1.0,
                Transform::Normal,
            ),
        ];
        let captured = CapturedOutputs {
            frames: vec![
                solid_frame(
                    "A",
                    8,
                    6,
                    RED,
                    FrameFormat::Xrgb8888,
                    Transform::Normal,
                    1.0,
                ),
                solid_frame(
                    "B",
                    8,
                    6,
                    GREEN,
                    FrameFormat::Xrgb8888,
                    Transform::Normal,
                    1.0,
                ),
            ],
            outputs,
        };
        // When stitching a 4x2 region spanning the boundary at x=8,
        let frame = captured
            .stitch(BackendKind::ExtImageCopyCapture, region(6.0, 2.0, 4.0, 2.0))
            .unwrap();
        // Then the composite is 4x2 with red left of the seam, green right.
        assert_eq!((frame.buffer.width, frame.buffer.height), (4, 2));
        assert_eq!(frame.buffer.pixel(1, 0), Some(RED));
        assert_eq!(frame.buffer.pixel(2, 0), Some(GREEN));
    }
}
