//! The `ext-image-copy-capture-v1` event sink: a plain-data state machine
//! filled by the [`Dispatch`](wayland_client::Dispatch) implementations and
//! turned into buffer parameters by pure functions, so format negotiation,
//! constraint completion, and failure mapping are unit-testable without a
//! live socket.

use bytes::BytesMut;
use flowshot_capture::{Frame, FrameBuffer, FrameFormat, OutputRef};
use flowshot_core::geometry::{OutputInfo, Transform};
use wayland_client::protocol::wl_shm;

use crate::error::{FrameFailure, IccError};

/// The `wl_shm` wire value of `ARGB8888` (little-endian `0xAARRGGBB`).
const SHM_ARGB8888: u32 = 0;
/// The `wl_shm` wire value of `XRGB8888` (little-endian `0x00RRGGBB`).
const SHM_XRGB8888: u32 = 1;
/// The `wl_shm` wire value of `RGBA8888` (byte order R, G, B, A).
const SHM_RGBA8888: u32 = 0x3432_4152;

/// Bytes per pixel for every v1 capture format (all are 32-bit).
const BYTES_PER_PIXEL: u32 = 4;

/// Format negotiation preference: opaque first (no alpha ambiguity for
/// screenshots), then premultiplied `ARGB`, then byte-order `RGBA`.
const FORMAT_PREFERENCE: [(u32, wl_shm::Format, FrameFormat); 3] = [
    (
        SHM_XRGB8888,
        wl_shm::Format::Xrgb8888,
        FrameFormat::Xrgb8888,
    ),
    (
        SHM_ARGB8888,
        wl_shm::Format::Argb8888,
        FrameFormat::Argb8888,
    ),
    (
        SHM_RGBA8888,
        wl_shm::Format::Rgba8888,
        FrameFormat::Rgba8888,
    ),
];

/// The allocation parameters derived from a session's buffer constraints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BufferParams {
    /// Buffer width in physical pixels (post-transform orientation).
    pub width: u32,
    /// Buffer height in physical pixels (post-transform orientation).
    pub height: u32,
    /// Bytes per row (tight: `width * 4`; v1 never pads).
    pub stride: u32,
    /// Total pool size in bytes (`stride * height`).
    pub size_bytes: usize,
    /// The negotiated `wl_shm` format to create the buffer with.
    pub shm_format: wl_shm::Format,
    /// The same format in the shared [`FrameFormat`] vocabulary.
    pub frame_format: FrameFormat,
}

/// Picks the preferred format among the advertised raw `wl_shm` values.
///
/// Returns the wire enum plus its [`FrameFormat`] equivalent, or `None` when
/// no v1-supported format was advertised.
pub(crate) fn select_format(advertised: &[u32]) -> Option<(wl_shm::Format, FrameFormat)> {
    FORMAT_PREFERENCE
        .into_iter()
        .find(|(wire, _, _)| advertised.contains(wire))
        .map(|(_, shm_format, frame_format)| (shm_format, frame_format))
}

/// Maps a `failure_reason` wire value onto the typed [`FrameFailure`].
///
/// Values outside the v1 enum become [`IccError::UnknownFailureReason`]
/// instead of silently degrading to `Unknown`.
pub(crate) fn failure_error(reason: u32) -> IccError {
    match reason {
        0 => IccError::FrameFailed {
            reason: FrameFailure::Unknown,
        },
        1 => IccError::FrameFailed {
            reason: FrameFailure::BufferConstraints,
        },
        2 => IccError::FrameFailed {
            reason: FrameFailure::Stopped,
        },
        other => IccError::UnknownFailureReason(other),
    }
}

/// The event sink of the capture currently in flight.
///
/// Filled exclusively by the dispatch implementations; read by the capture
/// runner between dispatch phases. `generation` tags the session and frame
/// proxies of the current capture so events from a destroyed predecessor
/// (still buffered in the socket when the next capture starts) cannot leak
/// into the new state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ActiveCapture {
    /// Tag carried by the current session/frame proxies' user data.
    pub generation: u32,
    /// Raw `wl_shm` format wire values from `shm_format` events, in arrival
    /// order (unknown values included, for diagnostics).
    pub shm_formats: Vec<u32>,
    /// Dimensions from the `buffer_size` event.
    pub buffer_size: Option<(u32, u32)>,
    /// Set by the session's `done` event: the constraint batch is complete.
    pub constraints_done: bool,
    /// Set by the session's `stopped` event.
    pub stopped: bool,
    /// The frame's `transform` event value (what the compositor applied to
    /// the buffer contents); diagnostic only - see the orientation rule in
    /// the runner.
    pub frame_transform: Option<u32>,
    /// Set by the frame's `ready` event.
    pub ready: bool,
    /// The frame's `failed` event reason wire value.
    pub failed: Option<u32>,
}

impl ActiveCapture {
    /// Resets the sink for a new capture and returns the new generation tag.
    pub(crate) fn begin(&mut self) -> u32 {
        self.generation = self.generation.wrapping_add(1);
        self.shm_formats.clear();
        self.buffer_size = None;
        self.constraints_done = false;
        self.stopped = false;
        self.frame_transform = None;
        self.ready = false;
        self.failed = None;
        self.generation
    }

    /// Whether the constraint phase settled: `done` or `stopped` arrived.
    pub(crate) fn constraints_settled(&self) -> bool {
        self.constraints_done || self.stopped
    }

    /// Whether the frame phase settled: `ready`, `failed`, or `stopped`.
    pub(crate) fn frame_settled(&self) -> bool {
        self.ready || self.failed.is_some() || self.stopped
    }

    /// Derives the allocation parameters from the collected constraints.
    ///
    /// # Errors
    ///
    /// [`IccError::SessionStopped`] when the session stopped,
    /// [`IccError::IncompleteConstraints`] when `done`/`buffer_size` are
    /// missing or a dimension is zero, [`IccError::NoSupportedFormat`] when
    /// format negotiation finds nothing usable, and
    /// [`IccError::Internal`] on arithmetic overflow.
    pub(crate) fn buffer_params(&self) -> Result<BufferParams, IccError> {
        if self.stopped {
            return Err(IccError::SessionStopped);
        }
        if !self.constraints_done {
            return Err(IccError::IncompleteConstraints);
        }
        let (width, height) = self.buffer_size.ok_or(IccError::IncompleteConstraints)?;
        if width == 0 || height == 0 {
            return Err(IccError::IncompleteConstraints);
        }
        let (shm_format, frame_format) =
            select_format(&self.shm_formats).ok_or_else(|| IccError::NoSupportedFormat {
                advertised: self.shm_formats.clone(),
            })?;
        let stride = width
            .checked_mul(BYTES_PER_PIXEL)
            .ok_or(IccError::Internal("buffer stride overflows u32"))?;
        let size_bytes = usize::try_from(stride)
            .ok()
            .and_then(|stride| stride.checked_mul(usize::try_from(height).ok()?))
            .ok_or(IccError::Internal("buffer size overflows usize"))?;
        Ok(BufferParams {
            width,
            height,
            stride,
            size_bytes,
            shm_format,
            frame_format,
        })
    }

    /// The outcome of a settled frame phase.
    ///
    /// # Errors
    ///
    /// The typed [`IccError`] for `failed(reason)` or `stopped`;
    /// [`IccError::Internal`] when called before the phase settled.
    pub(crate) fn frame_outcome(&self) -> Result<(), IccError> {
        if let Some(reason) = self.failed {
            return Err(failure_error(reason));
        }
        if self.stopped {
            return Err(IccError::SessionStopped);
        }
        if self.ready {
            return Ok(());
        }
        Err(IccError::Internal("frame outcome read before it settled"))
    }
}

/// Builds the [`Frame`] from the captured pixels: validates the buffer
/// geometry against the enumerated output and normalizes the orientation to
/// the shared contract (native pre-transform + transform metadata).
///
/// # Errors
///
/// [`IccError::BufferSizeMismatch`] when the session's buffer size disagrees
/// with the output's post-transform physical size, and
/// [`IccError::Geometry`] when the inverse remap rejects the buffer.
pub(crate) fn assemble_frame(
    pixels: &[u8],
    params: &BufferParams,
    info: &OutputInfo,
) -> Result<Frame, IccError> {
    let expected = info.buffer_size();
    let expected_width = u32::try_from(expected.width.0).unwrap_or(u32::MAX);
    let expected_height = u32::try_from(expected.height.0).unwrap_or(u32::MAX);
    if (params.width, params.height) != (expected_width, expected_height) {
        return Err(IccError::BufferSizeMismatch {
            reported: (params.width, params.height),
            expected: (expected.width.0, expected.height.0),
        });
    }
    let buffer = FrameBuffer {
        data: BytesMut::from(pixels),
        width: params.width,
        height: params.height,
        stride: params.stride,
        format: params.frame_format,
    };
    let buffer = to_native_orientation(buffer, info.transform)?;
    Ok(Frame {
        buffer,
        output: OutputRef::from(info),
        scale: info.scale,
        transform: info.transform,
    })
}

/// Inverse-remaps an upright (post-transform) buffer back to the output's
/// native orientation, per the orientation rule in the module docs.
///
/// # Errors
///
/// [`IccError::Internal`] on dimension conversion failures and
/// [`IccError::Geometry`] when the remap rejects the buffers.
fn to_native_orientation(
    buffer: FrameBuffer,
    transform: Transform,
) -> Result<FrameBuffer, IccError> {
    if transform == Transform::Normal {
        return Ok(buffer);
    }
    let inverse = transform.inverse();
    let (Ok(width), Ok(height)) = (
        usize::try_from(buffer.width),
        usize::try_from(buffer.height),
    ) else {
        return Err(IccError::Internal("buffer dimensions overflow usize"));
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
        .ok_or(IccError::Internal("native stride overflows u32"))?;
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

    fn params(width: u32, height: u32) -> BufferParams {
        BufferParams {
            width,
            height,
            stride: width * 4,
            size_bytes: usize::try_from(width * 4 * height).unwrap(),
            shm_format: wl_shm::Format::Xrgb8888,
            frame_format: FrameFormat::Xrgb8888,
        }
    }

    #[test]
    fn shm_format_wire_values_match_the_protocol_xml() {
        // The negotiation table compares raw wire values; pin the generated
        // enum discriminants to the wayland.xml numbers.
        assert_eq!(wl_shm::Format::Argb8888 as u32, 0);
        assert_eq!(wl_shm::Format::Xrgb8888 as u32, 1);
        assert_eq!(wl_shm::Format::Rgba8888 as u32, 0x3432_4152);
    }

    fn settled(formats: &[u32], size: (u32, u32)) -> ActiveCapture {
        ActiveCapture {
            generation: 1,
            shm_formats: formats.to_vec(),
            buffer_size: Some(size),
            constraints_done: true,
            ..ActiveCapture::default()
        }
    }

    #[test]
    fn format_preference_picks_xrgb_over_argb_over_rgba() {
        let (shm, frame) = select_format(&[SHM_ARGB8888, SHM_XRGB8888, SHM_RGBA8888]).unwrap();
        assert_eq!(shm, wl_shm::Format::Xrgb8888);
        assert_eq!(frame, FrameFormat::Xrgb8888);

        let (shm, frame) = select_format(&[SHM_RGBA8888, SHM_ARGB8888]).unwrap();
        assert_eq!(shm, wl_shm::Format::Argb8888);
        assert_eq!(frame, FrameFormat::Argb8888);

        let (_, frame) = select_format(&[SHM_RGBA8888]).unwrap();
        assert_eq!(frame, FrameFormat::Rgba8888);
    }

    #[test]
    fn unsupported_advertised_formats_negotiate_to_none() {
        // XBGR8888 and RGB565 only: no v1 FrameFormat equivalent.
        assert_eq!(select_format(&[0x3432_4258, 0x3631_4752]), None);
        assert_eq!(select_format(&[]), None);
    }

    #[test]
    fn params_carry_tight_stride_and_checked_size() {
        let params = settled(&[SHM_XRGB8888], (1920, 1080))
            .buffer_params()
            .unwrap();
        assert_eq!(params.stride, 7680);
        assert_eq!(params.size_bytes, 7680 * 1080);
        assert_eq!(params.frame_format, FrameFormat::Xrgb8888);
    }

    #[test]
    fn params_reject_zero_dimensions_and_missing_done() {
        let zero = settled(&[SHM_XRGB8888], (0, 1080));
        assert!(matches!(
            zero.buffer_params(),
            Err(IccError::IncompleteConstraints)
        ));
        let no_done = ActiveCapture {
            constraints_done: false,
            ..settled(&[SHM_XRGB8888], (64, 64))
        };
        assert!(matches!(
            no_done.buffer_params(),
            Err(IccError::IncompleteConstraints)
        ));
    }

    #[test]
    fn params_reject_unsupported_formats_naming_them() {
        let state = settled(&[0x3631_4752], (64, 64));
        match state.buffer_params() {
            Err(IccError::NoSupportedFormat { advertised }) => {
                assert_eq!(advertised, vec![0x3631_4752]);
            }
            other => panic!("expected NoSupportedFormat, got {other:?}"),
        }
    }

    #[test]
    fn stopped_session_short_circuits_params() {
        let state = ActiveCapture {
            stopped: true,
            ..settled(&[SHM_XRGB8888], (64, 64))
        };
        assert!(matches!(
            state.buffer_params(),
            Err(IccError::SessionStopped)
        ));
    }

    #[test]
    fn failure_reasons_map_onto_the_typed_table() {
        assert!(matches!(
            failure_error(0),
            IccError::FrameFailed {
                reason: FrameFailure::Unknown
            }
        ));
        assert!(matches!(
            failure_error(1),
            IccError::FrameFailed {
                reason: FrameFailure::BufferConstraints
            }
        ));
        assert!(matches!(
            failure_error(2),
            IccError::FrameFailed {
                reason: FrameFailure::Stopped
            }
        ));
        assert!(matches!(
            failure_error(7),
            IccError::UnknownFailureReason(7)
        ));
    }

    #[test]
    fn frame_outcome_orders_failed_over_stopped_over_ready() {
        let failed = ActiveCapture {
            failed: Some(1),
            stopped: true,
            ready: true,
            ..ActiveCapture::default()
        };
        assert!(matches!(
            failed.frame_outcome(),
            Err(IccError::FrameFailed {
                reason: FrameFailure::BufferConstraints
            })
        ));
        let stopped = ActiveCapture {
            stopped: true,
            ..ActiveCapture::default()
        };
        assert!(matches!(
            stopped.frame_outcome(),
            Err(IccError::SessionStopped)
        ));
        let ready = ActiveCapture {
            ready: true,
            ..ActiveCapture::default()
        };
        assert!(ready.frame_outcome().is_ok());
        let unsettled = ActiveCapture::default();
        assert!(matches!(
            unsettled.frame_outcome(),
            Err(IccError::Internal(_))
        ));
    }

    #[test]
    fn begin_resets_every_field_and_bumps_generation() {
        let mut state = ActiveCapture {
            generation: 41,
            failed: Some(2),
            ..settled(&[SHM_XRGB8888], (64, 64))
        };
        let generation = state.begin();
        assert_eq!(generation, 42);
        assert_eq!(
            state,
            ActiveCapture {
                generation: 42,
                ..ActiveCapture::default()
            }
        );
        assert!(!state.constraints_settled());
        assert!(!state.frame_settled());
    }

    #[test]
    fn settled_predicates_track_their_events() {
        let mut state = ActiveCapture::default();
        assert!(!state.constraints_settled());
        state.constraints_done = true;
        assert!(state.constraints_settled());
        assert!(!state.frame_settled());
        state.ready = true;
        assert!(state.frame_settled());
    }
    #[test]
    fn normal_orientation_passes_the_buffer_through() {
        let info = output_info(Transform::Normal, 2, 2);
        let pixels = vec![
            10u8, 11, 12, 13, 20, 21, 22, 23, 30, 31, 32, 33, 40, 41, 42, 43,
        ];
        let frame = assemble_frame(&pixels, &params(2, 2), &info).unwrap();
        assert_eq!(
            frame.buffer.data.as_ref(),
            &[
                10, 11, 12, 13, 20, 21, 22, 23, 30, 31, 32, 33, 40, 41, 42, 43
            ]
        );
        assert_eq!(frame.transform, Transform::Normal);
        assert_eq!(frame.output, OutputRef::Connector("TEST-1".to_owned()));
    }

    #[test]
    fn rotated_output_frames_are_normalized_to_native_orientation() {
        // Upright 3x2 buffer captured from a Rot90 output whose native
        // (pre-transform) size is 2x3.
        let info = output_info(Transform::Rot90, 2, 3);
        assert_eq!(
            (info.buffer_size().width.0, info.buffer_size().height.0),
            (3, 2)
        );
        // Upright rows: A B C / D E F (tag byte per pixel, padded to 4).
        let mut pixels = Vec::new();
        for row in [b"ABC", b"DEF"] {
            for tag in row {
                pixels.extend_from_slice(&[*tag, 0, 0, 0]);
            }
        }
        let frame = assemble_frame(&pixels, &params(3, 2), &info).unwrap();
        assert_eq!((frame.buffer.width, frame.buffer.height), (2, 3));
        assert_eq!(frame.transform, Transform::Rot90);
        // inverse(Rot90) = Rot270 maps upright (x, y) -> native (1 - y, x):
        // native rows: D A / E B / F C.
        let native: Vec<u8> = (0..6usize)
            .map(|index| frame.buffer.data[index * 4])
            .collect();
        assert_eq!(native, vec![b'D', b'A', b'E', b'B', b'F', b'C']);
        // The contract round-trip: applying the metadata transform restores
        // the upright image.
        let mut upright = vec![0u8; 24];
        frame
            .transform
            .remap_buffer(&frame.buffer.data, &mut upright, 2, 3, 4)
            .unwrap();
        let restored: Vec<u8> = (0..6usize).map(|index| upright[index * 4]).collect();
        assert_eq!(restored, vec![b'A', b'B', b'C', b'D', b'E', b'F']);
    }

    #[test]
    fn buffer_size_mismatch_is_a_typed_error() {
        let info = output_info(Transform::Normal, 4, 4);
        let err = assemble_frame(&[0u8; 16], &params(2, 2), &info).unwrap_err();
        match err {
            IccError::BufferSizeMismatch { reported, expected } => {
                assert_eq!(reported, (2, 2));
                assert_eq!(expected, (4, 4));
            }
            other => panic!("expected BufferSizeMismatch, got {other:?}"),
        }
    }
}
