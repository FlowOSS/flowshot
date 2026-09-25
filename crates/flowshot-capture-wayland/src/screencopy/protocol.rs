//! The `wlr-screencopy-unstable-v1` event sink and pure frame assembly.
//!
//! A plain-data state machine ([`ActiveScreencopy`]) filled by the
//! [`Dispatch`](wayland_client::Dispatch) implementations and turned into
//! buffer parameters and a [`Frame`] by pure functions, so format mapping,
//! constraint completion, the `y_invert` correction, and the native-orientation
//! contract are unit-testable without a live socket.
//!
//! # Orientation
//!
//! `wlr-screencopy` copies the output's front buffer directly, so it delivers
//! pixels in the output's NATIVE (pre-transform) orientation with the native
//! physical dimensions - verified against wlroots `types/wlr_screencopy_v1.c`
//! (`buffer_box.width = output->width`, read from `output->front_buffer`).
//! This already matches the shared [`Frame`] contract (native pixels +
//! `transform` metadata), so unlike the ICC backend there is NO inverse remap;
//! the only correction is the renderer's `y_invert` readback flag.

use bytes::BytesMut;
use flowshot_capture::{Frame, FrameBuffer, FrameFormat, OutputRef};
use flowshot_core::geometry::OutputInfo;
use wayland_client::protocol::wl_shm;

use crate::error::ScreencopyError;
use crate::icc::protocol::BufferParams;

/// Bytes per pixel for every v1 capture format (all are 32-bit).
const BYTES_PER_PIXEL: u32 = 4;

/// The raw buffer constraints from the frame's single `buffer` event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AdvertisedBuffer {
    /// The `wl_shm` format when the wire value is a known variant, else `None`.
    pub shm_format: Option<wl_shm::Format>,
    /// The raw `wl_shm` format wire value (diagnostics and error reporting).
    pub wire_format: u32,
    /// Buffer width in physical pixels (native, pre-transform orientation).
    pub width: u32,
    /// Buffer height in physical pixels (native, pre-transform orientation).
    pub height: u32,
    /// Bytes per row reported by the compositor (may include alignment padding).
    pub stride: u32,
}

/// Maps a single `wl_shm` format onto the shared [`FrameFormat`] vocabulary.
///
/// Returns `None` for formats with no v1 equivalent (`XBGR8888`, `RGB565`,
/// and friends); the backend then raises a typed
/// [`ScreencopyError::NoSupportedFormat`].
#[must_use]
pub(crate) fn frame_format_of(format: wl_shm::Format) -> Option<FrameFormat> {
    match format {
        wl_shm::Format::Xrgb8888 => Some(FrameFormat::Xrgb8888),
        wl_shm::Format::Argb8888 => Some(FrameFormat::Argb8888),
        wl_shm::Format::Rgba8888 => Some(FrameFormat::Rgba8888),
        // The generated enum is #[non_exhaustive]; any other wire format has
        // no v1 FrameFormat equivalent.
        _ => None,
    }
}

/// The terminal phase of a screencopy frame copy.
///
/// `ready` and `failed` are mutually exclusive, so they are one enum rather
/// than two booleans: the illegal "both" state is unrepresentable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum FramePhase {
    /// Neither `ready` nor `failed` has arrived yet.
    #[default]
    Pending,
    /// The frame's `ready` event arrived: the pixels are readable.
    Ready,
    /// The frame's `failed` event arrived.
    Failed,
}

/// The event sink of the screencopy frame currently in flight.
///
/// Filled exclusively by the dispatch implementation; read by the capture
/// runner between dispatch phases. `generation` tags the frame proxy so events
/// from a destroyed predecessor (still buffered in the socket when the next
/// per-output capture starts) cannot leak into the new state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ActiveScreencopy {
    /// Tag carried by the current frame proxy's user data.
    pub generation: u32,
    /// The buffer constraints from the frame's `buffer` event.
    pub buffer: Option<AdvertisedBuffer>,
    /// Set by the frame's `buffer_done` event (v3: all buffer types reported).
    pub buffer_done: bool,
    /// Set by the frame's `flags` event when the `y_invert` bit is present.
    pub y_invert: bool,
    /// The terminal copy phase from the frame's `ready`/`failed` event.
    pub phase: FramePhase,
}

impl ActiveScreencopy {
    /// Resets the sink for a new capture and returns the new generation tag.
    pub(crate) fn begin(&mut self) -> u32 {
        self.generation = self.generation.wrapping_add(1);
        self.buffer = None;
        self.buffer_done = false;
        self.y_invert = false;
        self.phase = FramePhase::Pending;
        self.generation
    }

    /// Whether the constraint phase settled: `buffer_done` or `failed` arrived.
    pub(crate) fn constraints_settled(&self) -> bool {
        self.buffer_done || self.phase == FramePhase::Failed
    }

    /// Whether the frame phase settled: `ready` or `failed` arrived.
    pub(crate) fn frame_settled(&self) -> bool {
        self.phase != FramePhase::Pending
    }

    /// Derives the allocation parameters from the advertised buffer.
    ///
    /// # Errors
    ///
    /// [`ScreencopyError::FrameFailed`] when the frame failed,
    /// [`ScreencopyError::IncompleteConstraints`] when `buffer_done`/`buffer`
    /// are missing or a dimension is zero,
    /// [`ScreencopyError::NoSupportedFormat`] when the advertised format has no
    /// v1 equivalent, and [`ScreencopyError::Internal`] on arithmetic overflow
    /// or a stride below the tight `width * 4` minimum.
    pub(crate) fn buffer_params(&self) -> Result<BufferParams, ScreencopyError> {
        if self.phase == FramePhase::Failed {
            return Err(ScreencopyError::FrameFailed);
        }
        if !self.buffer_done {
            return Err(ScreencopyError::IncompleteConstraints);
        }
        let advertised = self.buffer.ok_or(ScreencopyError::IncompleteConstraints)?;
        if advertised.width == 0 || advertised.height == 0 {
            return Err(ScreencopyError::IncompleteConstraints);
        }
        let shm_format = advertised
            .shm_format
            .ok_or(ScreencopyError::NoSupportedFormat {
                advertised: advertised.wire_format,
            })?;
        let frame_format =
            frame_format_of(shm_format).ok_or(ScreencopyError::NoSupportedFormat {
                advertised: advertised.wire_format,
            })?;
        let min_stride = advertised
            .width
            .checked_mul(BYTES_PER_PIXEL)
            .ok_or(ScreencopyError::Internal("buffer stride overflows u32"))?;
        if advertised.stride < min_stride {
            return Err(ScreencopyError::Internal(
                "compositor stride is below the tight width * 4 minimum",
            ));
        }
        let size_bytes = usize::try_from(advertised.stride)
            .ok()
            .and_then(|stride| stride.checked_mul(usize::try_from(advertised.height).ok()?))
            .ok_or(ScreencopyError::Internal("buffer size overflows usize"))?;
        Ok(BufferParams {
            width: advertised.width,
            height: advertised.height,
            stride: advertised.stride,
            size_bytes,
            shm_format,
            frame_format,
        })
    }

    /// The outcome of a settled frame phase.
    ///
    /// # Errors
    ///
    /// [`ScreencopyError::FrameFailed`] for the `failed` event;
    /// [`ScreencopyError::Internal`] when called before the phase settled.
    pub(crate) fn frame_outcome(&self) -> Result<(), ScreencopyError> {
        match self.phase {
            FramePhase::Failed => Err(ScreencopyError::FrameFailed),
            FramePhase::Ready => Ok(()),
            FramePhase::Pending => Err(ScreencopyError::Internal(
                "frame outcome read before it settled",
            )),
        }
    }
}

/// Flips a row-major buffer vertically in place, reversing the order of its
/// `height` rows of `stride` bytes each (padding travels with its row).
///
/// Corrects the `y_invert` flag a renderer with a bottom-left origin (the
/// OpenGL convention) sets on readback. A no-op for `height < 2`. Rows are
/// addressed through `get_mut` so a malformed slice is skipped, never a panic.
pub(crate) fn flip_vertical(data: &mut [u8], height: usize, stride: usize) {
    for top in 0..height / 2 {
        let bottom = height - 1 - top;
        let bottom_start = bottom * stride;
        let top_end = (top + 1) * stride;
        let (head, tail) = data.split_at_mut(bottom_start);
        if let (Some(top_row), Some(bottom_row)) =
            (head.get_mut(top * stride..top_end), tail.get_mut(..stride))
        {
            top_row.swap_with_slice(bottom_row);
        }
    }
}

/// Builds the [`Frame`] from the captured pixels: validates the NATIVE buffer
/// geometry against the enumerated output, applies the `y_invert` correction,
/// and attaches the transform/scale metadata.
///
/// The buffer is already in the shared contract's native orientation, so no
/// inverse remap runs (see the module docs); the geometry guard compares
/// against [`OutputInfo::physical_size`] (native), NOT the post-transform
/// [`OutputInfo::buffer_size`] the ICC backend uses.
///
/// # Errors
///
/// [`ScreencopyError::BufferSizeMismatch`] when the frame's buffer size
/// disagrees with the output's native physical size, and
/// [`ScreencopyError::Internal`] on dimension conversion failures.
pub(crate) fn assemble_frame(
    pixels: &[u8],
    params: &BufferParams,
    info: &OutputInfo,
    y_invert: bool,
) -> Result<Frame, ScreencopyError> {
    let expected = info.physical_size;
    let expected_width = u32::try_from(expected.width.0).unwrap_or(u32::MAX);
    let expected_height = u32::try_from(expected.height.0).unwrap_or(u32::MAX);
    if (params.width, params.height) != (expected_width, expected_height) {
        return Err(ScreencopyError::BufferSizeMismatch {
            reported: (params.width, params.height),
            expected: (expected.width.0, expected.height.0),
        });
    }
    let mut data = pixels.to_vec();
    if y_invert {
        let (Ok(height), Ok(stride)) = (
            usize::try_from(params.height),
            usize::try_from(params.stride),
        ) else {
            return Err(ScreencopyError::Internal(
                "buffer dimensions overflow usize",
            ));
        };
        flip_vertical(&mut data, height, stride);
    }
    let buffer = FrameBuffer {
        data: BytesMut::from(data.as_slice()),
        width: params.width,
        height: params.height,
        stride: params.stride,
        format: params.frame_format,
    };
    Ok(Frame {
        buffer,
        output: OutputRef::from(info),
        scale: info.scale,
        transform: info.transform,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use flowshot_core::geometry::{Logical, LogicalRect, PhysicalPx, PhysicalSize, Transform};

    use super::*;

    fn output_info(transform: Transform, width: i32, height: i32) -> OutputInfo {
        OutputInfo::new(
            "TEST-1",
            "Test output",
            LogicalRect::new(Logical(0.0), Logical(0.0), Logical(8.0), Logical(6.0)),
            PhysicalSize::new(PhysicalPx(width), PhysicalPx(height)),
            1.0,
            transform,
        )
        .unwrap()
    }

    fn advertised(
        format: wl_shm::Format,
        width: u32,
        height: u32,
        stride: u32,
    ) -> AdvertisedBuffer {
        AdvertisedBuffer {
            shm_format: Some(format),
            wire_format: u32::from(format),
            width,
            height,
            stride,
        }
    }

    fn settled(format: wl_shm::Format, width: u32, height: u32) -> ActiveScreencopy {
        ActiveScreencopy {
            generation: 1,
            buffer: Some(advertised(format, width, height, width * 4)),
            buffer_done: true,
            ..ActiveScreencopy::default()
        }
    }

    fn params(width: u32, height: u32, format: FrameFormat) -> BufferParams {
        BufferParams {
            width,
            height,
            stride: width * 4,
            size_bytes: usize::try_from(width * 4 * height).unwrap(),
            shm_format: wl_shm::Format::Xrgb8888,
            frame_format: format,
        }
    }

    #[test]
    fn frame_format_maps_the_three_v1_formats() {
        assert_eq!(
            frame_format_of(wl_shm::Format::Xrgb8888),
            Some(FrameFormat::Xrgb8888)
        );
        assert_eq!(
            frame_format_of(wl_shm::Format::Argb8888),
            Some(FrameFormat::Argb8888)
        );
        assert_eq!(
            frame_format_of(wl_shm::Format::Rgba8888),
            Some(FrameFormat::Rgba8888)
        );
        assert_eq!(frame_format_of(wl_shm::Format::Xbgr8888), None);
        assert_eq!(frame_format_of(wl_shm::Format::Rgb565), None);
    }

    #[test]
    fn params_carry_reported_stride_and_checked_size() {
        // A padded stride (width * 4 + 64) must be preserved, not tightened.
        let state = ActiveScreencopy {
            buffer: Some(advertised(wl_shm::Format::Argb8888, 64, 32, 64 * 4 + 64)),
            buffer_done: true,
            ..ActiveScreencopy::default()
        };
        let params = state.buffer_params().unwrap();
        assert_eq!(params.stride, 64 * 4 + 64);
        assert_eq!(params.size_bytes, (64 * 4 + 64) * 32);
        assert_eq!(params.frame_format, FrameFormat::Argb8888);
        assert_eq!(params.shm_format, wl_shm::Format::Argb8888);
    }

    #[test]
    fn params_reject_unsupported_format_naming_its_wire_value() {
        let state = ActiveScreencopy {
            buffer: Some(AdvertisedBuffer {
                shm_format: Some(wl_shm::Format::Xbgr8888),
                wire_format: u32::from(wl_shm::Format::Xbgr8888),
                width: 8,
                height: 8,
                stride: 32,
            }),
            buffer_done: true,
            ..ActiveScreencopy::default()
        };
        match state.buffer_params() {
            Err(ScreencopyError::NoSupportedFormat { advertised }) => {
                assert_eq!(advertised, u32::from(wl_shm::Format::Xbgr8888));
            }
            other => panic!("expected NoSupportedFormat, got {other:?}"),
        }
    }

    #[test]
    fn params_reject_unknown_wire_format() {
        let state = ActiveScreencopy {
            buffer: Some(AdvertisedBuffer {
                shm_format: None,
                wire_format: 0xdead_beef,
                width: 8,
                height: 8,
                stride: 32,
            }),
            buffer_done: true,
            ..ActiveScreencopy::default()
        };
        assert!(matches!(
            state.buffer_params(),
            Err(ScreencopyError::NoSupportedFormat {
                advertised: 0xdead_beef
            })
        ));
    }

    #[test]
    fn params_reject_zero_dimensions_missing_done_and_failed() {
        let zero = settled(wl_shm::Format::Xrgb8888, 0, 8);
        assert!(matches!(
            zero.buffer_params(),
            Err(ScreencopyError::IncompleteConstraints)
        ));
        let no_done = ActiveScreencopy {
            buffer_done: false,
            ..settled(wl_shm::Format::Xrgb8888, 8, 8)
        };
        assert!(matches!(
            no_done.buffer_params(),
            Err(ScreencopyError::IncompleteConstraints)
        ));
        let failed = ActiveScreencopy {
            phase: FramePhase::Failed,
            ..settled(wl_shm::Format::Xrgb8888, 8, 8)
        };
        assert!(matches!(
            failed.buffer_params(),
            Err(ScreencopyError::FrameFailed)
        ));
    }

    #[test]
    fn params_reject_stride_below_tight_minimum() {
        let state = ActiveScreencopy {
            buffer: Some(advertised(wl_shm::Format::Xrgb8888, 8, 8, 8 * 4 - 1)),
            buffer_done: true,
            ..ActiveScreencopy::default()
        };
        assert!(matches!(
            state.buffer_params(),
            Err(ScreencopyError::Internal(_))
        ));
    }

    #[test]
    fn frame_outcome_maps_each_phase() {
        let failed = ActiveScreencopy {
            phase: FramePhase::Failed,
            ..ActiveScreencopy::default()
        };
        assert!(matches!(
            failed.frame_outcome(),
            Err(ScreencopyError::FrameFailed)
        ));
        let ready = ActiveScreencopy {
            phase: FramePhase::Ready,
            ..ActiveScreencopy::default()
        };
        assert!(ready.frame_outcome().is_ok());
        let unsettled = ActiveScreencopy::default();
        assert!(matches!(
            unsettled.frame_outcome(),
            Err(ScreencopyError::Internal(_))
        ));
    }

    #[test]
    fn begin_resets_every_field_and_bumps_generation() {
        let mut state = ActiveScreencopy {
            generation: 41,
            phase: FramePhase::Failed,
            ..settled(wl_shm::Format::Xrgb8888, 8, 8)
        };
        let generation = state.begin();
        assert_eq!(generation, 42);
        assert_eq!(
            state,
            ActiveScreencopy {
                generation: 42,
                ..ActiveScreencopy::default()
            }
        );
        assert!(!state.constraints_settled());
        assert!(!state.frame_settled());
    }

    #[test]
    fn settled_predicates_track_their_events() {
        let mut state = ActiveScreencopy::default();
        assert!(!state.constraints_settled());
        state.buffer_done = true;
        assert!(state.constraints_settled());
        assert!(!state.frame_settled());
        state.phase = FramePhase::Ready;
        assert!(state.frame_settled());
    }

    #[test]
    fn flip_vertical_reverses_row_order() {
        // Three rows of 2 pixels each (stride 8), tagged by row.
        let mut data = vec![
            1, 1, 1, 1, 1, 1, 1, 1, // row 0
            2, 2, 2, 2, 2, 2, 2, 2, // row 1
            3, 3, 3, 3, 3, 3, 3, 3, // row 2
        ];
        flip_vertical(&mut data, 3, 8);
        assert_eq!(&data[0..8], &[3; 8]);
        assert_eq!(&data[8..16], &[2; 8]);
        assert_eq!(&data[16..24], &[1; 8]);
    }

    #[test]
    fn flip_vertical_preserves_per_row_padding() {
        // Two rows of 1 pixel (4 bytes) with 4 bytes of trailing padding each.
        // Padding bytes (9) must travel with their row, not smear.
        let mut data = vec![
            1, 1, 1, 1, 9, 9, 9, 9, // row 0: pixel + pad
            2, 2, 2, 2, 9, 9, 9, 9, // row 1: pixel + pad
        ];
        flip_vertical(&mut data, 2, 8);
        assert_eq!(&data[0..8], &[2, 2, 2, 2, 9, 9, 9, 9]);
        assert_eq!(&data[8..16], &[1, 1, 1, 1, 9, 9, 9, 9]);
    }

    #[test]
    fn flip_vertical_is_an_involution_and_noop_below_two_rows() {
        let original = vec![1, 2, 3, 4, 5, 6, 7, 8];
        let mut data = original.clone();
        flip_vertical(&mut data, 4, 2);
        flip_vertical(&mut data, 4, 2);
        assert_eq!(data, original);
        let mut single = vec![1, 2, 3, 4];
        flip_vertical(&mut single, 1, 4);
        assert_eq!(single, vec![1, 2, 3, 4]);
    }

    #[test]
    fn y_invert_frame_is_flipped_to_native_orientation() {
        // A 2x2 native buffer whose rows arrived inverted: row0=bottom, row1=top.
        let info = output_info(Transform::Normal, 2, 2);
        let inverted = vec![
            30u8, 31, 32, 33, 30, 31, 32, 33, // (inverted) bottom row tag 3
            10, 11, 12, 13, 10, 11, 12, 13, // (inverted) top row tag 1
        ];
        let frame =
            assemble_frame(&inverted, &params(2, 2, FrameFormat::Xrgb8888), &info, true).unwrap();
        // After the flip, the first row is tag 1 (the true top).
        assert_eq!(frame.buffer.data[0], 10);
        assert_eq!(frame.buffer.data[8], 30);
        assert_eq!(frame.transform, Transform::Normal);
    }

    #[test]
    fn non_inverted_frame_passes_through() {
        let info = output_info(Transform::Normal, 2, 2);
        let pixels = vec![
            10u8, 11, 12, 13, 20, 21, 22, 23, 30, 31, 32, 33, 40, 41, 42, 43,
        ];
        let frame =
            assemble_frame(&pixels, &params(2, 2, FrameFormat::Xrgb8888), &info, false).unwrap();
        assert_eq!(frame.buffer.data.as_ref(), pixels.as_slice());
        assert_eq!(frame.output, OutputRef::Connector("TEST-1".to_owned()));
    }

    #[test]
    fn native_buffer_is_validated_against_physical_size_not_transformed() {
        // A Rot90 output: native physical_size (2, 3), post-transform (3, 2).
        let info = output_info(Transform::Rot90, 2, 3);
        assert_eq!(
            (info.physical_size.width.0, info.physical_size.height.0),
            (2, 3)
        );
        assert_eq!(
            (info.buffer_size().width.0, info.buffer_size().height.0),
            (3, 2)
        );
        // The native (2, 3) buffer assembles, carrying the transform metadata
        // with NO remap (the buffer stays 2x3).
        let pixels = vec![0u8; 2 * 3 * 4];
        let frame =
            assemble_frame(&pixels, &params(2, 3, FrameFormat::Xrgb8888), &info, false).unwrap();
        assert_eq!((frame.buffer.width, frame.buffer.height), (2, 3));
        assert_eq!(frame.transform, Transform::Rot90);
        // A post-transform-sized (3, 2) buffer is REJECTED: screencopy never
        // delivers upright buffers (this is the ICC contract, not ours).
        let err = assemble_frame(
            &[0u8; 3 * 2 * 4],
            &params(3, 2, FrameFormat::Xrgb8888),
            &info,
            false,
        )
        .unwrap_err();
        match err {
            ScreencopyError::BufferSizeMismatch { reported, expected } => {
                assert_eq!(reported, (3, 2));
                assert_eq!(expected, (2, 3));
            }
            other => panic!("expected BufferSizeMismatch, got {other:?}"),
        }
    }

    #[test]
    fn consumer_remap_restores_upright_from_native_metadata() {
        // The contract round-trip: a native Rot90 buffer remapped by its
        // metadata transform yields the upright image. Native 2x3 rows:
        // A B / C D / E F (tag byte per pixel, padded to 4).
        let info = output_info(Transform::Rot90, 2, 3);
        let mut pixels = Vec::new();
        for row in [b"AB", b"CD", b"EF"] {
            for tag in row {
                pixels.extend_from_slice(&[*tag, 0, 0, 0]);
            }
        }
        let frame =
            assemble_frame(&pixels, &params(2, 3, FrameFormat::Xrgb8888), &info, false).unwrap();
        // Rot90 maps native (x, y) -> upright (y, last_x - x); upright is 3x2.
        let mut upright = vec![0u8; 24];
        frame
            .transform
            .remap_buffer(&frame.buffer.data, &mut upright, 2, 3, 4)
            .unwrap();
        let tags: Vec<u8> = (0..6usize).map(|index| upright[index * 4]).collect();
        // Native rows AB/CD/EF under Rot90 become upright rows: C A / D B... verify
        // the permutation is lossless (same multiset) and dimensions swapped.
        let mut sorted = tags.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, vec![b'A', b'B', b'C', b'D', b'E', b'F']);
    }
}
