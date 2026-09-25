//! Captured frame buffers and their placement metadata.
//!
//! Frames are CPU-side pixel buffers in v1. GPU-backed buffers (dmabuf and
//! platform equivalents) are a recorded future optimization and deliberately
//! absent from this contract.

use bytes::BytesMut;
use flowshot_core::geometry::{OutputInfo, Transform};
use serde::{Deserialize, Serialize};

/// Identifies the output a [`Frame`] came from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum OutputRef {
    /// The frame was captured from a single output, named by its connector
    /// (e.g. `DP-1`).
    Connector(String),
    /// The frame was stitched from one or more outputs in global layout space
    /// (region captures).
    Composite,
}

impl From<&OutputInfo> for OutputRef {
    fn from(output: &OutputInfo) -> Self {
        Self::Connector(output.connector.clone())
    }
}

/// The pixel layout of a [`FrameBuffer`].
///
/// Every v1 format is 32 bits per pixel; the names follow the little-endian
/// shared-memory conventions used by compositor protocols.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FrameFormat {
    /// Little-endian `u32` `0x00RRGGBB`: byte order B, G, R, unused X.
    Xrgb8888,
    /// Little-endian `u32` `0xAARRGGBB`: byte order B, G, R, A.
    Argb8888,
    /// Byte order R, G, B, A.
    Rgba8888,
}

impl FrameFormat {
    /// The number of bytes per pixel (4 for every v1 format).
    ///
    /// # Examples
    ///
    /// ```
    /// use flowshot_capture::FrameFormat;
    ///
    /// assert_eq!(FrameFormat::Rgba8888.bytes_per_pixel(), 4);
    /// ```
    #[must_use]
    pub const fn bytes_per_pixel(self) -> u32 {
        match self {
            Self::Xrgb8888 | Self::Argb8888 | Self::Rgba8888 => 4,
        }
    }
}

/// A CPU-side pixel buffer produced by a capture backend.
///
/// Invariant: `data` holds at least `stride * height` bytes and `stride` is at
/// least `width * format.bytes_per_pixel()` (row alignment may pad `stride`
/// beyond the tight minimum).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameBuffer {
    /// Row-major pixel data.
    pub data: BytesMut,
    /// Buffer width in physical pixels.
    pub width: u32,
    /// Buffer height in physical pixels.
    pub height: u32,
    /// Bytes per row, including any alignment padding.
    pub stride: u32,
    /// The pixel layout of `data`.
    pub format: FrameFormat,
}

impl FrameBuffer {
    /// The pixel at `(x, y)` as raw format bytes, or `None` when the
    /// coordinates fall outside the buffer or the data is too short.
    ///
    /// # Examples
    ///
    /// ```
    /// use bytes::BytesMut;
    /// use flowshot_capture::{FrameBuffer, FrameFormat};
    ///
    /// let buffer = FrameBuffer {
    ///     data: BytesMut::from(&[10u8, 20, 30, 40][..]),
    ///     width: 1,
    ///     height: 1,
    ///     stride: 4,
    ///     format: FrameFormat::Rgba8888,
    /// };
    /// assert_eq!(buffer.pixel(0, 0), Some([10, 20, 30, 40]));
    /// assert_eq!(buffer.pixel(1, 0), None);
    /// ```
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let stride = usize::try_from(self.stride).ok()?;
        let bpp = usize::try_from(self.format.bytes_per_pixel()).ok()?;
        let offset = usize::try_from(y)
            .ok()?
            .checked_mul(stride)?
            .checked_add(usize::try_from(x).ok()?.checked_mul(bpp)?)?;
        let bytes = self.data.get(offset..offset.checked_add(bpp)?)?;
        let mut pixel = [0u8; 4];
        pixel.copy_from_slice(&bytes[..4]);
        Some(pixel)
    }
}

/// One captured frame plus the metadata needed to place it in the global
/// logical layout.
///
/// Per-output frames (from
/// [`capture_outputs`](crate::CaptureBackend::capture_outputs)) carry the
/// output's native, pre-transform buffer orientation: consumers remap through
/// `transform` (see
/// [`Transform::remap_buffer`](flowshot_core::geometry::Transform::remap_buffer))
/// before display. Stitched region frames are already in layout orientation
/// and carry [`Transform::Normal`].
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    /// The pixel data.
    pub buffer: FrameBuffer,
    /// Which output (or layout composite) the frame belongs to.
    pub output: OutputRef,
    /// Scale factor converting this buffer's physical pixels to logical
    /// coordinates. Never an averaged value across outputs.
    pub scale: f64,
    /// The output transform the buffer was captured under.
    pub transform: Transform,
}
