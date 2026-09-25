//! Pure decision tables for portal screencast streams: pixel-format
//! mapping, buffer orientation, and stream-to-output matching.
//!
//! Portal implementations disagree on what a stream carries, so every
//! interpretation decision is an injected-value function here, table-tested
//! without a live compositor:
//!
//! - FORMAT: `PipeWire` negotiates one raw 32-bpp video format; the v1
//!   [`FrameFormat`] vocabulary covers `BGRx`/`BGRA`/`RGBx`/`RGBA`.
//! - ORIENTATION: `XDPH` captures through `wlr-screencopy` and delivers
//!   NATIVE (pre-transform) pixels; `mutter` composites and delivers
//!   UPRIGHT (post-transform) pixels. The buffer dimensions against the
//!   output's native and post-transform sizes decide which arrived.
//! - MATCHING: `XDPH` names the output in the stream's `mapping_id` and
//!   reports position `(0, 0)`; `mutter` reports logical positions. Match
//!   by mapping id first, then position, then unique size.

use flowshot_capture::FrameFormat;
use flowshot_core::geometry::OutputInfo;
use pipewire::spa::param::video::VideoFormat;

use super::error::PortalErrorKind;
use crate::stitch::round_to_i32;

/// The portal-reported metadata of one screencast stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StreamMeta {
    /// The `PipeWire` node id the portal assigned.
    pub node_id: u32,
    /// The implementation's source mapping id (`XDPH`: the output name).
    pub mapping_id: Option<String>,
    /// The stream position in compositor coordinate space.
    pub position: Option<(i32, i32)>,
    /// The stream size in compositor coordinate space.
    pub size: Option<(i32, i32)>,
}

/// How a negotiated `spa` video format maps onto the v1 frame vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MappedFormat {
    /// The frame format the raw bytes already satisfy.
    pub format: FrameFormat,
    /// `true` when the format's fourth byte is padding (`RGBx`) and must be
    /// forced opaque before the buffer is published as [`FrameFormat::Rgba8888`].
    pub force_opaque: bool,
}

/// Maps a negotiated `spa` video format onto the v1 frame vocabulary.
///
/// `VideoFormat` is an opaque wrapper over the C enum (not a Rust enum), so
/// unmatched values - including any future format - return `None` and the
/// caller raises [`PortalErrorKind::UnsupportedFormat`].
#[must_use]
pub(crate) fn frame_format_of(format: VideoFormat) -> Option<MappedFormat> {
    // Little-endian byte order B, G, R, x/A matches the Xrgb/Argb layouts.
    if format == VideoFormat::BGRx {
        Some(MappedFormat {
            format: FrameFormat::Xrgb8888,
            force_opaque: false,
        })
    } else if format == VideoFormat::BGRA {
        Some(MappedFormat {
            format: FrameFormat::Argb8888,
            force_opaque: false,
        })
    } else if format == VideoFormat::RGBx {
        Some(MappedFormat {
            format: FrameFormat::Rgba8888,
            force_opaque: true,
        })
    } else if format == VideoFormat::RGBA {
        Some(MappedFormat {
            format: FrameFormat::Rgba8888,
            force_opaque: false,
        })
    } else {
        None
    }
}

/// The orientation a captured stream buffer arrived in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Orientation {
    /// Native (pre-transform) pixels: the shared [`Frame`] contract as-is.
    Native,
    /// Upright (post-transform) pixels: need the inverse remap.
    Upright,
}

/// Decides which orientation a captured buffer of `dims` arrived in.
///
/// Dimension-swapping transforms (90/270 and flipped variants) are
/// unambiguous: native and upright sizes differ. For `Rot180`/flipped
/// transforms with equal dimensions the answer is implementation-defined
/// (`XDPH` native, `mutter` upright); v1 decides NATIVE, matching the
/// live-verified `XDPH` path - the `mutter` case is hardware-pending and
/// traced by the caller.
///
/// # Errors
///
/// [`PortalErrorKind::BufferSizeMismatch`] when the buffer matches neither
/// the output's native nor its post-transform physical size.
pub(crate) fn orientation_of(
    dims: (u32, u32),
    output: &OutputInfo,
) -> Result<Orientation, PortalErrorKind> {
    let native = size_pair(output.physical_size.width.0, output.physical_size.height.0);
    let upright_buffer = output.buffer_size();
    let upright = size_pair(upright_buffer.width.0, upright_buffer.height.0);
    if Some(dims) == native {
        return Ok(Orientation::Native);
    }
    if Some(dims) == upright {
        return Ok(Orientation::Upright);
    }
    Err(PortalErrorKind::BufferSizeMismatch {
        reported: dims,
        expected: (upright_buffer.width.0, upright_buffer.height.0),
    })
}

fn size_pair(width: i32, height: i32) -> Option<(u32, u32)> {
    let width = u32::try_from(width).ok()?;
    let height = u32::try_from(height).ok()?;
    Some((width, height))
}

/// Matches one stream to an output index, considering only outputs not yet
/// `claimed`.
///
/// Order: `mapping_id` against the connector (then output name), stream
/// position against the rounded logical origin, then a UNIQUE size match of
/// the captured frame against the output's post-transform or native size.
/// Ambiguity or no candidate returns `None`; the caller pairs a lone
/// remaining stream with a lone remaining output as the last resort.
#[must_use]
pub(crate) fn match_output(
    meta: &StreamMeta,
    frame_dims: (u32, u32),
    outputs: &[OutputInfo],
    claimed: &[bool],
) -> Option<usize> {
    let is_free = |index: usize| claimed.get(index).copied() == Some(false);
    if let Some(mapping) = &meta.mapping_id {
        let found = outputs
            .iter()
            .enumerate()
            .find(|(index, output)| {
                is_free(*index) && (output.connector == *mapping || output.name == *mapping)
            })
            .map(|(index, _)| index);
        if found.is_some() {
            return found;
        }
    }
    if let Some((x, y)) = meta.position {
        let found = outputs
            .iter()
            .enumerate()
            .find(|(index, output)| {
                is_free(*index)
                    && round_to_i32(output.logical_rect.x.0) == x
                    && round_to_i32(output.logical_rect.y.0) == y
            })
            .map(|(index, _)| index);
        if found.is_some() {
            return found;
        }
    }
    let size_matches: Vec<usize> = outputs
        .iter()
        .enumerate()
        .filter(|(index, output)| is_free(*index) && size_matches(output, frame_dims))
        .map(|(index, _)| index)
        .collect();
    if size_matches.len() == 1 {
        return size_matches.first().copied();
    }
    None
}

fn size_matches(output: &OutputInfo, dims: (u32, u32)) -> bool {
    let buffer = output.buffer_size();
    let physical = output.physical_size;
    size_pair(buffer.width.0, buffer.height.0) == Some(dims)
        || size_pair(physical.width.0, physical.height.0) == Some(dims)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use flowshot_core::geometry::{Logical, LogicalRect, PhysicalPx, PhysicalSize, Transform};

    use super::*;

    fn output(
        connector: &str,
        logical_origin: (f64, f64),
        physical: (i32, i32),
        transform: Transform,
    ) -> OutputInfo {
        let buffer = transform.apply_to_size(flowshot_core::geometry::Size::new(
            PhysicalPx(physical.0),
            PhysicalPx(physical.1),
        ));
        let logical_w = f64::from(buffer.width.0);
        let logical_h = f64::from(buffer.height.0);
        OutputInfo::new(
            connector,
            connector,
            LogicalRect::new(
                Logical(logical_origin.0),
                Logical(logical_origin.1),
                Logical(logical_w),
                Logical(logical_h),
            ),
            PhysicalSize::new(PhysicalPx(physical.0), PhysicalPx(physical.1)),
            1.0,
            transform,
        )
        .unwrap()
    }

    fn meta(node_id: u32, mapping_id: Option<&str>, position: Option<(i32, i32)>) -> StreamMeta {
        StreamMeta {
            node_id,
            mapping_id: mapping_id.map(str::to_owned),
            position,
            size: None,
        }
    }

    // ---- format mapping ----

    #[test]
    fn every_v1_spa_format_maps_onto_the_frame_vocabulary() {
        assert_eq!(
            frame_format_of(VideoFormat::BGRx),
            Some(MappedFormat {
                format: FrameFormat::Xrgb8888,
                force_opaque: false
            })
        );
        assert_eq!(
            frame_format_of(VideoFormat::BGRA),
            Some(MappedFormat {
                format: FrameFormat::Argb8888,
                force_opaque: false
            })
        );
        assert_eq!(
            frame_format_of(VideoFormat::RGBx),
            Some(MappedFormat {
                format: FrameFormat::Rgba8888,
                force_opaque: true
            })
        );
        assert_eq!(
            frame_format_of(VideoFormat::RGBA),
            Some(MappedFormat {
                format: FrameFormat::Rgba8888,
                force_opaque: false
            })
        );
    }

    #[test]
    fn non_rgb_formats_are_rejected() {
        assert_eq!(frame_format_of(VideoFormat::NV12), None);
        assert_eq!(frame_format_of(VideoFormat::Unknown), None);
    }

    // ---- orientation ----

    #[test]
    fn dimension_swap_decides_orientation_unambiguously() {
        let rotated = output("R", (0.0, 0.0), (1080, 1920), Transform::Rot90);
        // Native (pre-transform) 1080x1920 -> XDPH-style.
        assert_eq!(
            orientation_of((1080, 1920), &rotated).unwrap(),
            Orientation::Native
        );
        // Upright (post-transform) 1920x1080 -> mutter-style.
        assert_eq!(
            orientation_of((1920, 1080), &rotated).unwrap(),
            Orientation::Upright
        );
    }

    #[test]
    fn equal_dimensions_default_to_native() {
        let normal = output("N", (0.0, 0.0), (1920, 1080), Transform::Normal);
        assert_eq!(
            orientation_of((1920, 1080), &normal).unwrap(),
            Orientation::Native
        );
        let flipped = output("F", (0.0, 0.0), (1920, 1080), Transform::Rot180);
        assert_eq!(
            orientation_of((1920, 1080), &flipped).unwrap(),
            Orientation::Native
        );
    }

    #[test]
    fn foreign_dimensions_are_a_typed_mismatch() {
        let normal = output("N", (0.0, 0.0), (1920, 1080), Transform::Normal);
        let err = orientation_of((3840, 2160), &normal).unwrap_err();
        match err {
            PortalErrorKind::BufferSizeMismatch { reported, expected } => {
                assert_eq!(reported, (3840, 2160));
                assert_eq!(expected, (1920, 1080));
            }
            other => panic!("expected BufferSizeMismatch, got {other:?}"),
        }
    }

    // ---- stream matching ----

    #[test]
    fn mapping_id_wins_over_everything_else() {
        // XDPH reports position (0,0) for EVERY stream - the mapping id is
        // the only reliable key, and it must beat the bogus position.
        let outputs = vec![
            output("HDMI-A-1", (0.0, 0.0), (1920, 1080), Transform::Normal),
            output("DP-3", (1920.0, 0.0), (2560, 1440), Transform::Normal),
        ];
        let stream = meta(50, Some("DP-3"), Some((0, 0)));
        assert_eq!(
            match_output(&stream, (2560, 1440), &outputs, &[false, false]),
            Some(1)
        );
    }

    #[test]
    fn position_matches_the_logical_origin_without_mapping_id() {
        // mutter-style metadata: no mapping id, logical positions.
        let outputs = vec![
            output("A", (0.0, 0.0), (1920, 1080), Transform::Normal),
            output("B", (1920.0, 0.0), (1920, 1080), Transform::Normal),
        ];
        let stream = meta(7, None, Some((1920, 0)));
        assert_eq!(
            match_output(&stream, (1920, 1080), &outputs, &[false, false]),
            Some(1)
        );
    }

    #[test]
    fn unique_size_matches_without_metadata() {
        let outputs = vec![
            output("A", (0.0, 0.0), (1920, 1080), Transform::Normal),
            output("B", (1920.0, 0.0), (2560, 1440), Transform::Normal),
        ];
        let stream = meta(9, None, None);
        assert_eq!(
            match_output(&stream, (2560, 1440), &outputs, &[false, false]),
            Some(1)
        );
    }

    #[test]
    fn ambiguous_size_without_metadata_matches_nothing() {
        // Two identical outputs, no mapping id, no position: guessing would
        // silently swap monitors, so the match refuses.
        let outputs = vec![
            output("A", (0.0, 0.0), (1920, 1080), Transform::Normal),
            output("B", (1920.0, 0.0), (1920, 1080), Transform::Normal),
        ];
        let stream = meta(9, None, None);
        assert_eq!(
            match_output(&stream, (1920, 1080), &outputs, &[false, false]),
            None
        );
    }

    #[test]
    fn claimed_outputs_are_never_rematched() {
        let outputs = vec![
            output("A", (0.0, 0.0), (1920, 1080), Transform::Normal),
            output("B", (1920.0, 0.0), (2560, 1440), Transform::Normal),
        ];
        let stream = meta(50, Some("DP-3"), None);
        let unmatched = meta(51, None, None);
        assert_eq!(
            match_output(&stream, (2560, 1440), &outputs, &[false, false]),
            Some(1)
        );
        // B is claimed; the only free output (A, 1920x1080) does not match
        // the 2560x1440 frame, so nothing matches.
        let wrong_size = meta(52, Some("DP-3"), None);
        assert_eq!(
            match_output(&wrong_size, (2560, 1440), &outputs, &[false, true]),
            None
        );
        assert_eq!(
            match_output(&unmatched, (1920, 1080), &outputs, &[false, true]),
            Some(0)
        );
    }
}
