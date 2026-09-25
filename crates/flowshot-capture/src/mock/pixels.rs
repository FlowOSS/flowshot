//! Pixel plumbing for [`MockBackend`](super::MockBackend): fixture decoding,
//! nearest-neighbour blitting, transform remapping, and the synthetic cursor
//! glyph. Deterministic by construction so downstream tests can assert exact
//! pixel values.

use bytes::BytesMut;
use flowshot_core::geometry::{OutputInfo, Transform};

use crate::error::CaptureError;
use crate::frame::{FrameBuffer, FrameFormat};
use crate::kind::BackendKind;

/// The embedded fixture PNGs, cycled for outputs beyond the default layout.
/// The primary fixture is solid red, the secondary solid green, so stitched
/// frames are pixel-assertable.
const FIXTURES: [&[u8]; 2] = [
    include_bytes!("../fixtures/mock_output_primary.png"),
    include_bytes!("../fixtures/mock_output_secondary.png"),
];

/// Origin of the synthetic cursor glyph painted when `paint_cursor` is set.
const CURSOR_ORIGIN: (u32, u32) = (4, 4);
/// Edge length of the synthetic cursor glyph, in physical pixels.
const CURSOR_SIZE: u32 = 3;

/// An immutable rectangular view into an RGBA buffer, in pixels.
pub(super) struct RgbaView<'a> {
    pub(super) data: &'a [u8],
    /// Row width of the whole buffer.
    pub(super) width: usize,
    pub(super) x: usize,
    pub(super) y: usize,
    pub(super) rect_width: usize,
    pub(super) rect_height: usize,
}

/// A mutable rectangular view into an RGBA buffer, in pixels.
pub(super) struct RgbaViewMut<'a> {
    pub(super) data: &'a mut [u8],
    /// Row width of the whole buffer.
    pub(super) width: usize,
    pub(super) x: usize,
    pub(super) y: usize,
    pub(super) rect_width: usize,
    pub(super) rect_height: usize,
}

/// Nearest-neighbour samples the source rect into the destination rect.
///
/// Invariant: both views are consistent with their buffers
/// (`(y + rect_height) * width * 4 <= data.len()`) and every coordinate is in
/// bounds; callers build views from the core crop algebra, which clamps to
/// buffer bounds.
pub(super) fn blit_nearest(source: &RgbaView<'_>, destination: &mut RgbaViewMut<'_>) {
    for row in 0..destination.rect_height {
        let source_row = source.y + row * source.rect_height / destination.rect_height;
        for column in 0..destination.rect_width {
            let source_column = source.x + column * source.rect_width / destination.rect_width;
            let source_offset = (source_row * source.width + source_column) * 4;
            let destination_offset =
                ((destination.y + row) * destination.width + destination.x + column) * 4;
            destination.data[destination_offset..destination_offset + 4]
                .copy_from_slice(&source.data[source_offset..source_offset + 4]);
        }
    }
}

/// Paints the synthetic cursor glyph (a white block at [`CURSOR_ORIGIN`]) so
/// `paint_cursor` is observable in frame data.
pub(super) fn paint_cursor_glyph(buffer: &mut FrameBuffer) {
    let (Ok(stride), Ok(bytes_per_pixel)) = (
        usize::try_from(buffer.stride),
        usize::try_from(buffer.format.bytes_per_pixel()),
    ) else {
        return;
    };
    for glyph_row in 0..CURSOR_SIZE {
        for glyph_column in 0..CURSOR_SIZE {
            let x = CURSOR_ORIGIN.0 + glyph_column;
            let y = CURSOR_ORIGIN.1 + glyph_row;
            if x >= buffer.width || y >= buffer.height {
                continue;
            }
            let (Ok(x), Ok(y)) = (usize::try_from(x), usize::try_from(y)) else {
                continue;
            };
            let offset = y * stride + x * bytes_per_pixel;
            if let Some(pixel) = buffer.data.get_mut(offset..offset + 4) {
                pixel.copy_from_slice(&[255, 255, 255, 255]);
            }
        }
    }
}

/// Rounds a logical edge to an integer pixel edge (half away from zero),
/// saturating at the `i32` bounds; non-finite input rounds to `0`.
pub(super) fn round_to_i32(value: f64) -> i32 {
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

/// Decodes the fixture PNG for output `index` into a native-orientation
/// (pre-transform) RGBA buffer sized `output.physical_size`.
pub(super) fn fixture_native(
    kind: BackendKind,
    index: usize,
    output: &OutputInfo,
) -> Result<FrameBuffer, CaptureError> {
    let decoded = image::load_from_memory(FIXTURES[index % FIXTURES.len()])
        .map_err(|err| CaptureError::Decode {
            backend: kind,
            source: err.into(),
        })?
        .to_rgba8();
    let width = u32::try_from(output.physical_size.width.0)
        .map_err(|_| decode_error(kind, "output physical width is negative"))?;
    let height = u32::try_from(output.physical_size.height.0)
        .map_err(|_| decode_error(kind, "output physical height is negative"))?;
    let data = if (decoded.width(), decoded.height()) == (width, height) {
        decoded.into_raw()
    } else {
        resample(kind, &decoded, width, height)?
    };
    let stride = width
        .checked_mul(4)
        .ok_or_else(|| decode_error(kind, "fixture stride overflows u32"))?;
    Ok(FrameBuffer {
        data: BytesMut::from(data.as_slice()),
        width,
        height,
        stride,
        format: FrameFormat::Rgba8888,
    })
}

/// Nearest-neighbour resamples a decoded RGBA image to the target size.
fn resample(
    kind: BackendKind,
    decoded: &image::RgbaImage,
    target_width: u32,
    target_height: u32,
) -> Result<Vec<u8>, CaptureError> {
    let source_width = usize_dim(kind, decoded.width())?;
    let source_height = usize_dim(kind, decoded.height())?;
    let target_w = usize_dim(kind, target_width)?;
    let target_h = usize_dim(kind, target_height)?;
    let pixels = target_w
        .checked_mul(target_h)
        .ok_or_else(|| decode_error(kind, "fixture dimensions overflow usize"))?;
    let mut data = vec![
        0u8;
        pixels.checked_mul(4).ok_or_else(|| decode_error(
            kind,
            "fixture dimensions overflow usize"
        ))?
    ];
    blit_nearest(
        &RgbaView {
            data: decoded.as_raw(),
            width: source_width,
            x: 0,
            y: 0,
            rect_width: source_width,
            rect_height: source_height,
        },
        &mut RgbaViewMut {
            data: &mut data,
            width: target_w,
            x: 0,
            y: 0,
            rect_width: target_w,
            rect_height: target_h,
        },
    );
    Ok(data)
}

/// Remaps a native-orientation buffer into post-transform orientation.
pub(super) fn oriented(
    kind: BackendKind,
    buffer: FrameBuffer,
    transform: Transform,
) -> Result<FrameBuffer, CaptureError> {
    if transform == Transform::Normal {
        return Ok(buffer);
    }
    let width = usize_dim(kind, buffer.width)?;
    let height = usize_dim(kind, buffer.height)?;
    let mut data = vec![0u8; buffer.data.len()];
    transform
        .remap_buffer(&buffer.data, &mut data, width, height, 4)
        .map_err(|err| CaptureError::Backend {
            backend: kind,
            source: err.into(),
        })?;
    let (width, height) = if transform.swaps_dimensions() {
        (buffer.height, buffer.width)
    } else {
        (buffer.width, buffer.height)
    };
    Ok(FrameBuffer {
        data: BytesMut::from(data.as_slice()),
        width,
        height,
        stride: width
            .checked_mul(4)
            .ok_or_else(|| internal_error(kind, "oriented stride overflows u32"))?,
        format: buffer.format,
    })
}

pub(super) fn usize_dim(kind: BackendKind, value: u32) -> Result<usize, CaptureError> {
    usize::try_from(value).map_err(|_| internal_error(kind, "dimension overflows usize"))
}

pub(super) fn decode_error(kind: BackendKind, message: &str) -> CaptureError {
    CaptureError::Decode {
        backend: kind,
        source: message.to_owned().into(),
    }
}

pub(super) fn internal_error(kind: BackendKind, message: &str) -> CaptureError {
    CaptureError::Backend {
        backend: kind,
        source: message.to_owned().into(),
    }
}
