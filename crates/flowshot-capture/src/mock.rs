//! A fixture-driven [`CaptureBackend`] for downstream tests.
//!
//! [`MockBackend`] serves embedded PNG fixtures as frames, synthesizes a
//! cursor track, and stitches region captures through the real
//! [`OutputLayout`] crop algebra from `flowshot-core` - so downstream tests
//! exercise the same physical-first placement math as production backends.

mod pixels;

use bytes::BytesMut;
use flowshot_core::geometry::{
    Logical, LogicalPoint, LogicalRect, OutputInfo, OutputLayout, PhysicalPoint, PhysicalPx,
    PhysicalSize, Transform,
};
use futures::stream;

use crate::backend::{CaptureBackend, CaptureOpts, PermissionResult};
use crate::cursor::{CursorEvent, CursorStream};
use crate::error::CaptureError;
use crate::frame::{Frame, FrameBuffer, FrameFormat, OutputRef};
use crate::kind::BackendKind;
use async_trait::async_trait;
use pixels::{RgbaView, RgbaViewMut, blit_nearest, paint_cursor_glyph, round_to_i32};

/// Number of `Moved` events in the synthetic cursor track.
const CURSOR_TRACK_STEPS: i32 = 8;

/// A capture backend serving embedded PNG fixtures.
///
/// The default layout is two abutting outputs in global logical space:
///
/// | connector | logical rect   | physical | scale | fixture |
/// |-----------|----------------|----------|-------|---------|
/// | `MOCK-1`  | (0, 0, 8, 6)   | 8 x 6    | 1.0   | red     |
/// | `MOCK-2`  | (8, 0, 4, 3)   | 8 x 6    | 2.0   | green   |
///
/// Frames are delivered as [`FrameFormat::Rgba8888`]. Region captures are
/// stitched at scale 1.0 (one destination pixel per logical unit), sampling
/// each output's post-transform buffer with nearest-neighbour filtering -
/// deterministic and assertable, not visually pretty.
#[derive(Debug, Clone)]
pub struct MockBackend {
    kind: BackendKind,
    outputs: Vec<OutputInfo>,
    permission: PermissionResult,
}

impl MockBackend {
    /// Creates a mock backend with the default two-output fixture layout.
    ///
    /// # Examples
    ///
    /// ```
    /// use flowshot_capture::{BackendKind, CaptureBackend, MockBackend};
    ///
    /// let backend = MockBackend::new(BackendKind::ExtImageCopyCapture);
    /// assert_eq!(backend.kind(), BackendKind::ExtImageCopyCapture);
    /// ```
    #[must_use]
    pub fn new(kind: BackendKind) -> Self {
        Self {
            kind,
            outputs: Self::default_outputs(),
            permission: PermissionResult::NotRequired,
        }
    }

    /// Replaces the output layout (builder style).
    ///
    /// Fixtures are cycled by output index and resampled to each output's
    /// physical size, so arbitrary layouts work.
    #[must_use]
    pub fn with_outputs(mut self, outputs: Vec<OutputInfo>) -> Self {
        self.outputs = outputs;
        self
    }

    /// Replaces the permission result (builder style).
    #[must_use]
    pub fn with_permission(mut self, permission: PermissionResult) -> Self {
        self.permission = permission;
        self
    }

    /// The default two-output fixture layout, for tests that need the layout
    /// itself (e.g. to build a matching [`OutputLayout`]).
    ///
    /// [`OutputLayout`]: flowshot_core::geometry::OutputLayout
    #[must_use]
    pub fn default_outputs() -> Vec<OutputInfo> {
        vec![
            OutputInfo {
                connector: "MOCK-1".to_owned(),
                name: "Mock primary".to_owned(),
                logical_rect: LogicalRect::new(
                    Logical(0.0),
                    Logical(0.0),
                    Logical(8.0),
                    Logical(6.0),
                ),
                physical_size: PhysicalSize::new(PhysicalPx(8), PhysicalPx(6)),
                scale: 1.0,
                transform: Transform::Normal,
            },
            OutputInfo {
                connector: "MOCK-2".to_owned(),
                name: "Mock secondary".to_owned(),
                logical_rect: LogicalRect::new(
                    Logical(8.0),
                    Logical(0.0),
                    Logical(4.0),
                    Logical(3.0),
                ),
                physical_size: PhysicalSize::new(PhysicalPx(8), PhysicalPx(6)),
                scale: 2.0,
                transform: Transform::Normal,
            },
        ]
    }
}

#[async_trait]
impl CaptureBackend for MockBackend {
    fn kind(&self) -> BackendKind {
        self.kind
    }

    async fn outputs(&self) -> Result<Vec<OutputInfo>, CaptureError> {
        Ok(self.outputs.clone())
    }

    async fn capture_outputs(&self, opts: CaptureOpts) -> Result<Vec<Frame>, CaptureError> {
        let mut frames = Vec::with_capacity(self.outputs.len());
        for (index, output) in self.outputs.iter().enumerate() {
            let mut buffer = pixels::fixture_native(self.kind, index, output)?;
            if opts.paint_cursor {
                paint_cursor_glyph(&mut buffer);
            }
            frames.push(Frame {
                buffer,
                output: OutputRef::from(output),
                scale: output.scale,
                transform: output.transform,
            });
        }
        Ok(frames)
    }

    async fn capture_region(&self, region: LogicalRect) -> Result<Frame, CaptureError> {
        let layout = OutputLayout::new(self.outputs.clone());
        let clamped = layout
            .clamp_region_to_layout(region)
            .ok_or(CaptureError::RegionOutsideLayout { region })?;
        let (width, height) =
            stitched_size(&clamped).ok_or(CaptureError::RegionOutsideLayout { region })?;
        let destination_width = pixels::usize_dim(self.kind, width)?;
        let destination_height = pixels::usize_dim(self.kind, height)?;
        let pixels_count = destination_width
            .checked_mul(destination_height)
            .ok_or_else(|| pixels::internal_error(self.kind, "region dimensions overflow usize"))?;
        let mut data = vec![
            0u8;
            pixels_count.checked_mul(4).ok_or_else(|| {
                pixels::internal_error(self.kind, "region dimensions overflow usize")
            })?
        ];

        for (index, output) in self.outputs.iter().enumerate() {
            let Some(logical) = output.logical_rect.intersection(&clamped) else {
                continue;
            };
            let Some(physical) = output.physical_crop(clamped) else {
                continue;
            };
            let native = pixels::fixture_native(self.kind, index, output)?;
            let source = pixels::oriented(self.kind, native, output.transform)?;
            blit_crop(
                self.kind,
                &source,
                physical,
                DestinationRect::of(&logical, &clamped),
                &mut data,
                destination_width,
            )?;
        }

        Ok(Frame {
            buffer: FrameBuffer {
                data: BytesMut::from(data.as_slice()),
                width,
                height,
                stride: width.checked_mul(4).ok_or_else(|| {
                    pixels::internal_error(self.kind, "region stride overflows u32")
                })?,
                format: FrameFormat::Rgba8888,
            },
            output: OutputRef::Composite,
            scale: 1.0,
            transform: Transform::Normal,
        })
    }

    fn cursor_events(&self) -> Option<CursorStream> {
        let track = [CursorEvent::Entered]
            .into_iter()
            .chain((0..CURSOR_TRACK_STEPS).map(|step| CursorEvent::Moved {
                position: LogicalPoint::new(Logical(f64::from(step)), Logical(f64::from(step))),
            }))
            .chain([CursorEvent::Hotspot {
                offset: PhysicalPoint::zero(),
            }])
            .chain([CursorEvent::Left]);
        Some(Box::pin(stream::iter(track)))
    }

    async fn request_permission(&self) -> PermissionResult {
        self.permission
    }
}

/// The stitched destination size for a clamped region at scale 1.0, or `None`
/// when it rounds to zero pixels.
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

/// Blits one output's physical crop into the stitched destination buffer.
fn blit_crop(
    kind: BackendKind,
    source: &FrameBuffer,
    physical: flowshot_core::geometry::PhysicalRect,
    destination: DestinationRect,
    data: &mut [u8],
    destination_width: usize,
) -> Result<(), CaptureError> {
    let edge = |value: i32, what: &str| {
        usize::try_from(value).map_err(|_| pixels::internal_error(kind, what))
    };
    blit_nearest(
        &RgbaView {
            data: &source.data,
            width: pixels::usize_dim(kind, source.width)?,
            x: edge(physical.x.0, "crop rect is negative")?,
            y: edge(physical.y.0, "crop rect is negative")?,
            rect_width: edge(physical.width.0, "crop rect is negative")?,
            rect_height: edge(physical.height.0, "crop rect is negative")?,
        },
        &mut RgbaViewMut {
            data,
            width: destination_width,
            x: edge(destination.x, "destination rect overflows usize")?,
            y: edge(destination.y, "destination rect overflows usize")?,
            rect_width: edge(destination.width, "destination rect overflows usize")?,
            rect_height: edge(destination.height, "destination rect overflows usize")?,
        },
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    // float_cmp: fixture scales are exact literals (1.0, 2.0); strict
    // comparison is the intent.
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

    use futures::StreamExt;

    use super::*;

    const RED: [u8; 4] = [255, 0, 0, 255];
    const GREEN: [u8; 4] = [0, 255, 0, 255];
    const WHITE: [u8; 4] = [255, 255, 255, 255];

    fn logical(x: f64, y: f64, width: f64, height: f64) -> LogicalRect {
        LogicalRect::new(Logical(x), Logical(y), Logical(width), Logical(height))
    }

    #[tokio::test]
    async fn outputs_returns_the_default_fixture_layout() {
        let backend = MockBackend::new(BackendKind::ExtImageCopyCapture);
        let outputs = backend.outputs().await.unwrap();
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[0].connector, "MOCK-1");
        assert_eq!(outputs[0].scale, 1.0);
        assert_eq!(outputs[1].connector, "MOCK-2");
        assert_eq!(outputs[1].scale, 2.0);
        let layout = OutputLayout::new(outputs);
        assert_eq!(
            layout.union_bounds(),
            Some(logical(0.0, 0.0, 12.0, 6.0)),
            "mixed-scale layout must union in logical space"
        );
    }

    #[tokio::test]
    async fn capture_outputs_yields_one_native_frame_per_output() {
        let backend = MockBackend::new(BackendKind::WlrScreencopy);
        let frames = backend
            .capture_outputs(CaptureOpts::new(false))
            .await
            .unwrap();
        assert_eq!(frames.len(), 2);

        let primary = &frames[0];
        assert_eq!(primary.output, OutputRef::Connector("MOCK-1".to_owned()));
        assert_eq!((primary.buffer.width, primary.buffer.height), (8, 6));
        assert_eq!(primary.buffer.stride, 32);
        assert_eq!(primary.buffer.format, FrameFormat::Rgba8888);
        assert_eq!(primary.scale, 1.0);
        assert_eq!(primary.transform, Transform::Normal);
        assert_eq!(primary.buffer.pixel(0, 0), Some(RED));

        let secondary = &frames[1];
        assert_eq!(secondary.output, OutputRef::Connector("MOCK-2".to_owned()));
        assert_eq!(secondary.scale, 2.0, "per-output scale, never averaged");
        assert_eq!(secondary.buffer.pixel(7, 5), Some(GREEN));
    }

    #[tokio::test]
    async fn paint_cursor_composites_the_glyph_into_every_frame() {
        let backend = MockBackend::new(BackendKind::ExtImageCopyCapture);
        let painted = backend
            .capture_outputs(CaptureOpts::new(true))
            .await
            .unwrap();
        let plain = backend
            .capture_outputs(CaptureOpts::new(false))
            .await
            .unwrap();

        assert_ne!(painted[0].buffer, plain[0].buffer);
        assert_eq!(painted[0].buffer.pixel(4, 4), Some(WHITE));
        assert_eq!(painted[0].buffer.pixel(6, 5), Some(WHITE));
        assert_eq!(plain[0].buffer.pixel(4, 4), Some(RED));
        // Outside the glyph the fixture is untouched.
        assert_eq!(painted[0].buffer.pixel(0, 0), Some(RED));
    }

    #[tokio::test]
    async fn capture_region_within_one_output_samples_that_fixture() {
        let backend = MockBackend::new(BackendKind::KwinScreenShot2);
        let frame = backend
            .capture_region(logical(1.0, 1.0, 3.0, 2.0))
            .await
            .unwrap();
        assert_eq!((frame.buffer.width, frame.buffer.height), (3, 2));
        assert_eq!(frame.output, OutputRef::Composite);
        assert_eq!(frame.scale, 1.0);
        assert_eq!(frame.transform, Transform::Normal);
        for y in 0..2 {
            for x in 0..3 {
                assert_eq!(frame.buffer.pixel(x, y), Some(RED));
            }
        }
    }

    #[tokio::test]
    async fn capture_region_spanning_outputs_stitches_both_fixtures() {
        let backend = MockBackend::new(BackendKind::PortalScreenCast);
        // Spans MOCK-1 (logical x 6..8, scale 1) and MOCK-2 (x 8..10, scale 2).
        let frame = backend
            .capture_region(logical(6.0, 0.0, 4.0, 3.0))
            .await
            .unwrap();
        assert_eq!((frame.buffer.width, frame.buffer.height), (4, 3));
        assert_eq!(frame.buffer.pixel(1, 1), Some(RED));
        assert_eq!(frame.buffer.pixel(2, 1), Some(GREEN));
        assert_eq!(frame.buffer.pixel(3, 2), Some(GREEN));
    }

    #[tokio::test]
    async fn capture_region_outside_the_layout_is_a_typed_error() {
        let backend = MockBackend::new(BackendKind::WlrScreencopy);
        let err = backend
            .capture_region(logical(100.0, 100.0, 10.0, 10.0))
            .await
            .unwrap_err();
        assert!(matches!(err, CaptureError::RegionOutsideLayout { .. }));
        assert!(err.to_string().contains("does not intersect any output"));
    }

    #[tokio::test]
    async fn capture_region_on_an_empty_layout_is_a_typed_error() {
        let backend = MockBackend::new(BackendKind::WlrScreencopy).with_outputs(Vec::new());
        assert!(
            backend
                .capture_outputs(CaptureOpts::default())
                .await
                .unwrap()
                .is_empty()
        );
        let err = backend
            .capture_region(logical(0.0, 0.0, 4.0, 4.0))
            .await
            .unwrap_err();
        assert!(matches!(err, CaptureError::RegionOutsideLayout { .. }));
    }

    #[tokio::test]
    async fn capture_region_handles_a_rotated_output() {
        // Physical 6x8 rotated 90 degrees -> post-transform buffer 8x6,
        // matching the logical rect at scale 1.
        let rotated = OutputInfo::new(
            "MOCK-R",
            "Mock rotated",
            logical(0.0, 0.0, 8.0, 6.0),
            PhysicalSize::new(PhysicalPx(6), PhysicalPx(8)),
            1.0,
            Transform::Rot90,
        )
        .unwrap();
        let backend =
            MockBackend::new(BackendKind::ExtImageCopyCapture).with_outputs(vec![rotated.clone()]);

        // Per-output frames stay in native orientation and carry the transform.
        let frames = backend
            .capture_outputs(CaptureOpts::new(false))
            .await
            .unwrap();
        assert_eq!((frames[0].buffer.width, frames[0].buffer.height), (6, 8));
        assert_eq!(frames[0].transform, Transform::Rot90);

        // Region frames are delivered in layout orientation.
        let frame = backend
            .capture_region(logical(2.0, 2.0, 2.0, 2.0))
            .await
            .unwrap();
        assert_eq!((frame.buffer.width, frame.buffer.height), (2, 2));
        assert_eq!(frame.transform, Transform::Normal);
        assert_eq!(frame.buffer.pixel(0, 0), Some(RED));
        assert_eq!(frame.buffer.pixel(1, 1), Some(RED));
    }

    #[tokio::test]
    async fn custom_output_sizes_resample_the_fixture() {
        let large = OutputInfo::new(
            "MOCK-L",
            "Mock large",
            logical(0.0, 0.0, 8.0, 6.0),
            PhysicalSize::new(PhysicalPx(16), PhysicalPx(12)),
            2.0,
            Transform::Normal,
        )
        .unwrap();
        let backend = MockBackend::new(BackendKind::PortalScreenshot).with_outputs(vec![large]);
        let frames = backend
            .capture_outputs(CaptureOpts::new(false))
            .await
            .unwrap();
        assert_eq!((frames[0].buffer.width, frames[0].buffer.height), (16, 12));
        assert_eq!(frames[0].buffer.pixel(15, 11), Some(RED));
        assert_eq!(frames[0].scale, 2.0);
    }

    #[tokio::test]
    async fn cursor_stream_delivers_the_synthetic_track() {
        let backend = MockBackend::new(BackendKind::ExtImageCopyCapture);
        let events: Vec<CursorEvent> = backend.cursor_events().unwrap().collect().await;
        assert_eq!(events.first(), Some(&CursorEvent::Entered));
        assert_eq!(events.last(), Some(&CursorEvent::Left));
        let moved: Vec<LogicalPoint> = events
            .iter()
            .filter_map(|event| match event {
                CursorEvent::Moved { position } => Some(*position),
                _ => None,
            })
            .collect();
        assert_eq!(moved.len(), usize::try_from(CURSOR_TRACK_STEPS).unwrap());
        assert!(moved.windows(2).all(|pair| pair[1].x > pair[0].x));
        assert!(events.contains(&CursorEvent::Hotspot {
            offset: PhysicalPoint::zero(),
        }));
    }

    #[tokio::test]
    async fn permission_defaults_to_not_required_and_is_configurable() {
        let backend = MockBackend::new(BackendKind::WlrScreencopy);
        assert_eq!(
            backend.request_permission().await,
            PermissionResult::NotRequired
        );
        let denied = backend.with_permission(PermissionResult::Denied);
        assert_eq!(denied.request_permission().await, PermissionResult::Denied);
        assert!(!denied.request_permission().await.can_capture());
    }

    #[tokio::test]
    async fn mock_is_usable_as_a_trait_object() {
        let backend: Box<dyn CaptureBackend> =
            Box::new(MockBackend::new(BackendKind::PortalScreenCast));
        assert_eq!(backend.kind(), BackendKind::PortalScreenCast);
        let frames = backend
            .capture_outputs(CaptureOpts::default())
            .await
            .unwrap();
        assert_eq!(frames.len(), 2);
    }
}
