//! The pixel capture path: per-output root-window `GetImage` reads with the
//! `MIT-SHM` fd-passing fast path.
//!
//! # Transport
//!
//! The correctness base is core-protocol [`xproto::get_image`] on the root
//! window (`Z_PIXMAP`, all planes): X screen coordinates are framebuffer pixels,
//! so one read per output's screen rect returns its pixels 1:1
//! (physical-pixels-first, ADR-001). When the server provides `MIT-SHM` >= 1.2
//! (the version that added fd-passing: `AttachFd`), the fast path replaces the
//! socket round-trip of the pixel payload: `memfd_create` + `ftruncate` +
//! [`shm::attach_fd`] + [`shm::get_image`], with the readback through ordinary
//! file I/O - the `flowshot-capture-wayland` `memfd` pattern, zero `unsafe`,
//! no `SysV` `shmat`. Any fast-path failure degrades to the plain read
//! (logged), never fails the capture.
//!
//! # Pixel format
//!
//! Phase A supports root depth 24 only (32 bpp `Z_PIXMAP`); anything else is
//! the typed [`X11Error::UnsupportedDepth`]. Depth-24 `Z_PIXMAP` pixels are
//! `0x00RRGGBB` words in the server's image byte order; with the negotiated
//! `LSBFirst` order (every little-endian platform) the memory byte order is
//! B, G, R, X - exactly the [`FrameFormat::Xrgb8888`] contract, verified
//! against a live solid-color oracle (learnings notepad). `MSBFirst` servers
//! (big-endian platforms) are the typed [`X11Error::UnsupportedByteOrder`].
//! Stride is tight: `width * 4` (`Z_PIXMAP` scanlines of 32-bit pixels need no
//! padding).
//!
//! # Orientation
//!
//! A root-window read returns SCREEN-space pixels: post-transform, upright as
//! displayed. The shared [`Frame`] contract wants the output's NATIVE
//! (pre-transform) orientation with the transform as metadata, so a rotated
//! or flipped output's pixels are remapped through
//! [`Transform::inverse`] before assembly - the exact inverse of the
//! Wayland backends' native-buffer delivery, and the stitcher's
//! `oriented()` remap undoes it for region captures. The cursor is composited
//! BEFORE the inverse remap (screen space is where the cursor image lives),
//! so it rotates with the output content.
//!
//! [`xproto::get_image`]: x11rb::protocol::xproto::get_image
//! [`shm::attach_fd`]: x11rb::protocol::shm::attach_fd
//! [`shm::get_image`]: x11rb::protocol::shm::get_image
//! [`Transform::inverse`]: flowshot_core::geometry::Transform::inverse

use std::fs::File;
use std::io::Read;
use std::time::Instant;

use bytes::BytesMut;
use flowshot_capture::{CaptureOpts, Frame, FrameBuffer, FrameFormat, OutputRef};
use flowshot_core::geometry::Transform;
use nix::sys::memfd::{MemFdCreateFlag, memfd_create};
use nix::unistd::ftruncate;
use x11rb::connection::Connection;
use x11rb::cookie::{Cookie, VoidCookie};
use x11rb::errors::ReplyError;
use x11rb::protocol::shm;
use x11rb::protocol::xproto::{self, ImageFormat, ImageOrder};

use crate::connect::X11Connection;
use crate::cursor::{XrgbCanvas, composite_cursor_xrgb, cursor_snapshot};
use crate::error::X11Error;
use crate::output::{MonitorData, MonitoredOutput, enumerate};
use crate::probe::{ExtensionVersion, X11Caps};
use crate::stitch::CapturedOutputs;

/// The only root depth Phase A captures (24-bit pixels in 32 bpp `Z_PIXMAP`).
const SUPPORTED_ROOT_DEPTH: u8 = 24;

/// Bytes per pixel in every v1 frame format (and in depth-24 `Z_PIXMAP`).
const BYTES_PER_PIXEL: usize = 4;

/// Stride bytes per pixel, as the `u32` wire field wants it.
const STRIDE_BYTES_PER_PIXEL: u32 = 4;

/// The `MIT-SHM` version that added fd-passing (`AttachFd`): servers below it
/// only offer `SysV` `shmattach`, which this crate deliberately never uses
/// (it cannot be done without `unsafe`).
const SHM_FD_PASSING_MAJOR: u32 = 1;

/// See [`SHM_FD_PASSING_MAJOR`].
const SHM_FD_PASSING_MINOR: u32 = 2;

/// Name of the anonymous capture file, visible in `/proc/<pid>/fd`.
const MEMFD_NAME: &std::ffi::CStr = c"flowshot-x11-capture";

/// Runs one full capture pass: every lit output, one [`Frame`] per output in
/// enumeration order, cursor composited when `opts.paint_cursor` and XFIXES
/// is available.
///
/// # Errors
///
/// [`X11Error::UnsupportedDepth`] / [`X11Error::UnsupportedByteOrder`] when
/// the server's pixel format is outside Phase A, [`X11Error::Protocol`] when
/// a request fails on the wire (the SHM fast path excepted - it degrades to
/// the plain read), and propagates the enumeration and `MIT-SHM` allocation
/// errors.
pub(crate) fn capture_run(
    conn: &X11Connection,
    caps: X11Caps,
    opts: CaptureOpts,
) -> Result<CapturedOutputs, X11Error> {
    validate_pixel_format(conn)?;
    let monitors = enumerate(conn)?;
    // The MIT-SHM spec wants the version negotiated on THIS connection
    // before extension requests (Xorg serves AttachFd without it, other
    // servers may not); one extra round-trip per capture run.
    let shm_ready = shm_fast_path(caps) && negotiate_shm(conn);
    let cursor = opts
        .paint_cursor
        .then(|| cursor_snapshot(conn, caps))
        .flatten();
    let mut frames = Vec::with_capacity(monitors.len());
    let mut outputs = Vec::with_capacity(monitors.len());
    for monitor in &monitors {
        let mut pixels = grab_screen(conn, shm_ready, &monitor.data)?;
        if let Some(snapshot) = &cursor {
            composite_cursor_xrgb(
                XrgbCanvas {
                    data: &mut pixels,
                    width: monitor.data.width,
                    height: monitor.data.height,
                    origin: (monitor.data.x, monitor.data.y),
                },
                snapshot,
            );
        }
        frames.push(assemble_frame(pixels, monitor)?);
        outputs.push(monitor.info.clone());
    }
    Ok(CapturedOutputs { outputs, frames })
}

/// Negotiates the `MIT-SHM` version on this connection and reports whether
/// the server confirmed fd-passing support (>= 1.2). Any failure means "plain
/// `GetImage` for this run" (logged, never an error).
fn negotiate_shm(conn: &X11Connection) -> bool {
    let negotiated = shm::query_version(conn.conn())
        .map_err(ReplyError::from)
        .and_then(Cookie::reply);
    match negotiated {
        Ok(reply) => ExtensionVersion::new(
            u32::from(reply.major_version),
            u32::from(reply.minor_version),
        )
        .at_least(SHM_FD_PASSING_MAJOR, SHM_FD_PASSING_MINOR),
        Err(error) => {
            tracing::debug!(%error, "MIT-SHM negotiation failed on the capture connection; using plain GetImage");
            false
        }
    }
}

/// Gates the capture on the server's pixel format: root depth 24 and
/// `LSBFirst` image byte order (the only combination the Phase A
/// [`FrameFormat::Xrgb8888`] mapping covers).
///
/// # Errors
///
/// [`X11Error::UnsupportedDepth`] for any other root depth,
/// [`X11Error::UnsupportedByteOrder`] for `MSBFirst`, and
/// [`X11Error::Internal`] when the setup lacks the connected screen.
fn validate_pixel_format(conn: &X11Connection) -> Result<(), X11Error> {
    let setup = conn.conn().setup();
    if setup.image_byte_order != ImageOrder::LSB_FIRST {
        return Err(X11Error::UnsupportedByteOrder);
    }
    let depth = setup
        .roots
        .get(conn.screen())
        .ok_or(X11Error::Internal(
            "the X server's setup does not contain the screen it reported at connect",
        ))?
        .root_depth;
    if depth != SUPPORTED_ROOT_DEPTH {
        return Err(X11Error::UnsupportedDepth { depth });
    }
    Ok(())
}

/// Whether the `MIT-SHM` fast path is usable: the server advertised
/// fd-passing support (>= 1.2) at capability-query time.
fn shm_fast_path(caps: X11Caps) -> bool {
    caps.shm
        .is_some_and(|version| version.at_least(SHM_FD_PASSING_MAJOR, SHM_FD_PASSING_MINOR))
}

/// Reads one output's screen rect: the `MIT-SHM` fd-passing fast path when
/// it was negotiated on this connection, degrading to the plain socket read
/// on any fast-path failure.
fn grab_screen(
    conn: &X11Connection,
    shm_ready: bool,
    monitor: &MonitorData,
) -> Result<Vec<u8>, X11Error> {
    let started = Instant::now();
    let (pixels, path) = if shm_ready {
        match grab_shm(conn, monitor) {
            Ok(pixels) => (pixels, "shm"),
            Err(error) => {
                tracing::debug!(%error, "MIT-SHM capture failed; falling back to plain GetImage");
                (grab_plain(conn, monitor)?, "plain-fallback")
            }
        }
    } else {
        (grab_plain(conn, monitor)?, "plain")
    };
    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    tracing::debug!(
        connector = %monitor.connector,
        path,
        elapsed_ms,
        "captured X11 output"
    );
    Ok(pixels)
}

/// The fast path: an anonymous file sized for the output, handed to the
/// server by descriptor, filled by `shm::GetImage`, and read back through
/// ordinary file I/O (both mappings go through the same page cache - no
/// memory mapping, no `unsafe`).
fn grab_shm(conn: &X11Connection, monitor: &MonitorData) -> Result<Vec<u8>, X11Error> {
    let size = expected_len(monitor);
    let fd =
        memfd_create(MEMFD_NAME, MemFdCreateFlag::MFD_CLOEXEC).map_err(std::io::Error::from)?;
    let length = i64::try_from(size)
        .map_err(|_| X11Error::Internal("capture size overflows the file length"))?;
    ftruncate(&fd, length).map_err(std::io::Error::from)?;
    let file = File::from(fd);
    let seg = conn
        .conn()
        .generate_id()
        .map_err(|_| X11Error::Internal("the X connection exhausted its resource ids"))?;
    // The server dups the descriptor out of the socket; x11rb owns (and
    // closes) the clone it sends, the local file stays open for the readback.
    shm::attach_fd(conn.conn(), seg, file.try_clone()?, false)?.check()?;
    let reply = shm::get_image(
        conn.conn(),
        conn.root(),
        monitor.x,
        monitor.y,
        monitor.width,
        monitor.height,
        !0,
        u8::from(ImageFormat::Z_PIXMAP),
        seg,
        0,
    )?
    .reply()?;
    if usize::try_from(reply.size).unwrap_or(usize::MAX) != size {
        return Err(X11Error::ImageSizeMismatch {
            expected: size,
            actual: usize::try_from(reply.size).unwrap_or(usize::MAX),
        });
    }
    let mut pixels = vec![0u8; size];
    let mut handle = &file;
    handle.read_exact(&mut pixels)?;
    let detached = shm::detach(conn.conn(), seg)
        .map_err(ReplyError::from)
        .and_then(VoidCookie::check);
    if let Err(error) = detached {
        tracing::debug!(%error, "MIT-SHM detach failed; the connection close will clean up");
    }
    Ok(pixels)
}

/// The correctness base: core-protocol `GetImage` on the root window, pixels
/// delivered over the socket.
fn grab_plain(conn: &X11Connection, monitor: &MonitorData) -> Result<Vec<u8>, X11Error> {
    let reply = xproto::get_image(
        conn.conn(),
        ImageFormat::Z_PIXMAP,
        conn.root(),
        monitor.x,
        monitor.y,
        monitor.width,
        monitor.height,
        !0,
    )?
    .reply()?;
    Ok(reply.data)
}

/// Assembles the shared [`Frame`] from one output's screen-space pixels:
/// length validation, cursor-free (the caller composites first), the inverse
/// transform remap into native orientation, and the placement metadata.
///
/// # Errors
///
/// [`X11Error::ImageSizeMismatch`] when `pixels` is shorter than the output's
/// screen geometry requires, and [`X11Error::Geometry`] when the remap
/// rejects the buffers.
fn assemble_frame(pixels: Vec<u8>, monitor: &MonitoredOutput) -> Result<Frame, X11Error> {
    let data = &monitor.data;
    let expected = expected_len(data);
    if pixels.len() < expected {
        return Err(X11Error::ImageSizeMismatch {
            expected,
            actual: pixels.len(),
        });
    }
    let (screen_width, screen_height) = (usize::from(data.width), usize::from(data.height));
    let transform = monitor.info.transform;
    let native = if transform == Transform::Normal {
        pixels
    } else {
        let mut native = vec![0u8; expected];
        transform.inverse().remap_buffer(
            &pixels,
            &mut native,
            screen_width,
            screen_height,
            BYTES_PER_PIXEL,
        )?;
        native
    };
    let (width, height) = if transform.swaps_dimensions() {
        (u32::from(data.height), u32::from(data.width))
    } else {
        (u32::from(data.width), u32::from(data.height))
    };
    Ok(Frame {
        buffer: FrameBuffer {
            data: BytesMut::from(native.as_slice()),
            width,
            height,
            stride: width * STRIDE_BYTES_PER_PIXEL,
            format: FrameFormat::Xrgb8888,
        },
        output: OutputRef::from(&monitor.info),
        scale: monitor.info.scale,
        transform,
    })
}

/// The byte length one output's screen rect requires at 32 bpp.
fn expected_len(monitor: &MonitorData) -> usize {
    usize::from(monitor.width) * usize::from(monitor.height) * BYTES_PER_PIXEL
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    // Fixture scales are exact binary fractions and fixture geometry is
    // integral (geometry-notepad convention).
    #![allow(clippy::float_cmp)]

    use x11rb::protocol::randr::Rotation;

    use super::*;

    fn monitor_data(width: u16, height: u16, rotation: Rotation) -> MonitorData {
        MonitorData {
            connector: "TEST-1".to_owned(),
            x: 0,
            y: 0,
            width,
            height,
            mm_width: 0,
            mm_height: 0,
            rotation,
        }
    }

    fn monitored(width: u16, height: u16, rotation: Rotation, scale: f64) -> MonitoredOutput {
        let data = monitor_data(width, height, rotation);
        MonitoredOutput {
            info: data.to_output_info(Some(scale * 96.0)).unwrap(),
            data,
        }
    }

    /// Screen-space pixels where every pixel encodes its own coordinates:
    /// bytes [x, y, 0, 0] - so a remap moves verifiable values.
    fn coordinate_pixels(width: u16, height: u16) -> Vec<u8> {
        let mut pixels = Vec::with_capacity(usize::from(width) * usize::from(height) * 4);
        for y in 0..height {
            for x in 0..width {
                pixels.extend_from_slice(&[
                    u8::try_from(x).unwrap(),
                    u8::try_from(y).unwrap(),
                    0,
                    0,
                ]);
            }
        }
        pixels
    }

    #[test]
    fn unrotated_output_assembles_tightly_strided_xrgb_frame() {
        let monitor = monitored(4, 3, Rotation::ROTATE0, 1.0);
        let frame = assemble_frame(coordinate_pixels(4, 3), &monitor).unwrap();
        assert_eq!((frame.buffer.width, frame.buffer.height), (4, 3));
        assert_eq!(frame.buffer.stride, 16, "tight stride: width * 4");
        assert_eq!(frame.buffer.format, FrameFormat::Xrgb8888);
        assert_eq!(frame.buffer.data.len(), 48);
        assert_eq!(frame.output, OutputRef::Connector("TEST-1".to_owned()));
        assert_eq!(frame.scale, 1.0);
        assert_eq!(frame.transform, Transform::Normal);
        // Pixel (2, 1) kept its coordinate encoding: no remap ran.
        assert_eq!(frame.buffer.pixel(2, 1), Some([2, 1, 0, 0]));
    }

    #[test]
    fn rotated_output_is_remapped_into_native_orientation() {
        // Screen space is 3 wide x 4 tall (a 4x3 panel under Rot90); the
        // frame must come out native 4x3 with Transform::Rot90 metadata.
        let monitor = monitored(3, 4, Rotation::ROTATE90, 1.0);
        assert_eq!(monitor.info.transform, Transform::Rot90);
        let frame = assemble_frame(coordinate_pixels(3, 4), &monitor).unwrap();
        assert_eq!((frame.buffer.width, frame.buffer.height), (4, 3));
        assert_eq!(frame.transform, Transform::Rot90);
        // Inverse(Rot90) = Rot270 maps screen (x, y) -> native (3 - 1 - y, x)
        // (Rot270's map_point: (x, y) -> (last_y - y, x) with last_y = 3).
        // Screen pixel (2, 1) [bytes 2,1] lands at native (3 - 1, 2) = (2, 2).
        assert_eq!(frame.buffer.pixel(2, 2), Some([2, 1, 0, 0]));
        // Round trip: re-applying the transform recovers screen orientation
        // (this is what the stitcher's oriented() does for region captures).
        let mut upright = vec![0u8; 48];
        monitor
            .info
            .transform
            .remap_buffer(&frame.buffer.data, &mut upright, 4, 3, 4)
            .unwrap();
        assert_eq!(&upright[..48], &coordinate_pixels(3, 4)[..48]);
    }

    #[test]
    fn short_pixel_buffer_is_a_typed_size_mismatch() {
        let monitor = monitored(4, 3, Rotation::ROTATE0, 1.0);
        let err = assemble_frame(vec![0u8; 47], &monitor).unwrap_err();
        match err {
            X11Error::ImageSizeMismatch { expected, actual } => {
                assert_eq!((expected, actual), (48, 47));
            }
            other => panic!("expected ImageSizeMismatch, got {other:?}"),
        }
    }

    #[test]
    fn rotated_scale_two_output_keeps_per_output_metadata() {
        let monitor = monitored(3, 4, Rotation::ROTATE90, 2.0);
        let frame = assemble_frame(coordinate_pixels(3, 4), &monitor).unwrap();
        assert_eq!(frame.scale, 2.0, "the output's own scale, never averaged");
        assert_eq!(frame.buffer.width, 4);
        // Native size matches OutputInfo.physical_size (pre-transform).
        assert_eq!(
            (
                monitor.info.physical_size.width.0,
                monitor.info.physical_size.height.0
            ),
            (4, 3)
        );
    }

    #[test]
    fn shm_gate_accepts_fd_passing_versions_only() {
        use crate::probe::ExtensionVersion;
        let caps_with = |shm: Option<ExtensionVersion>| X11Caps {
            randr: ExtensionVersion::new(1, 5),
            xfixes: None,
            shm,
        };
        assert!(shm_fast_path(caps_with(Some(ExtensionVersion::new(1, 2)))));
        assert!(shm_fast_path(caps_with(Some(ExtensionVersion::new(1, 3)))));
        assert!(shm_fast_path(caps_with(Some(ExtensionVersion::new(2, 0)))));
        assert!(!shm_fast_path(caps_with(Some(ExtensionVersion::new(1, 1)))));
        assert!(!shm_fast_path(caps_with(Some(ExtensionVersion::new(1, 0)))));
        assert!(!shm_fast_path(caps_with(None)));
    }

    #[test]
    fn expected_len_is_tight_32bpp() {
        assert_eq!(
            expected_len(&monitor_data(2880, 1620, Rotation::ROTATE0)),
            2880 * 1620 * 4
        );
        assert_eq!(expected_len(&monitor_data(1, 1, Rotation::ROTATE0)), 4);
    }
}
