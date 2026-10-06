//! One-shot cursor support: position via `XQueryPointer`, image via XFIXES
//! `GetCursorImage`, and the pure compositing math.
//!
//! X11 offers no cursor *stream* - Phase A reads the cursor exactly once per
//! capture ([`cursor_pos`], [`cursor_snapshot`]), which is why
//! [`CaptureBackend::cursor_events`](flowshot_capture::CaptureBackend::cursor_events)
//! is `None` on this backend (a documented degradation, never a failure).
//!
//! # Position space
//!
//! `QueryPointer` reports the pointer in X screen coordinates: physical,
//! post-transform pixels - the same orientation as the global logical layout
//! (which is also post-transform), so the conversion divides by the scale of
//! the output under the pointer and adds that output's logical origin; the
//! output transform cancels and never enters the math. XFIXES reports the
//! cursor position as the on-screen *hotspot*; the image's top-left corner is
//! `(x - xhot, y - yhot)` (the convention every XFIXES consumer - `OBS`,
//! `GStreamer`, Weylus - applies).
//!
//! # Pixel format
//!
//! `GetCursorImage` pixels are `CARD32` values `0xAARRGGBB` (alpha in the
//! most-significant byte); this module converts them to byte order R, G, B, A
//! so the compositing math matches the Wayland crate's
//! `composite_cursor_rgba` contract.

use flowshot_core::geometry::{LogicalPoint, PhysicalPoint, ToLogical};
use x11rb::cookie::Cookie;
use x11rb::errors::ReplyError;
use x11rb::protocol::xfixes::{self, GetCursorImageReply};
use x11rb::protocol::xproto;

use crate::connect::X11Connection;
use crate::error::X11Error;
use crate::output::{MonitoredOutput, enumerate};
use crate::probe::{X11Caps, XFIXES_CLIENT_VERSION};

/// A captured cursor image ready for compositing: `RGBA8888` pixels plus the
/// screen-space top-left corner (hotspot position minus hotspot offset).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CursorSnapshot {
    /// The image's top-left corner in X screen coordinates (physical).
    pub top_left: PhysicalPoint,
    /// Row-major `RGBA8888` pixels (`width * height * 4` bytes).
    pub rgba: Vec<u8>,
    /// Image width in physical pixels.
    pub width: u32,
    /// Image height in physical pixels.
    pub height: u32,
}

/// Reads the one-shot cursor position in global logical layout space.
///
/// Returns `None` when the pointer is on another screen of a multi-screen
/// display or sits inside no output (a gap in the layout) - callers degrade
/// (no cursor-aware preselect), they never fail.
///
/// # Errors
///
/// Propagates the [`X11Error`] of the `QueryPointer` request or the RANDR
/// enumeration.
pub fn cursor_pos(conn: &X11Connection) -> Result<Option<LogicalPoint>, X11Error> {
    let reply = xproto::query_pointer(conn.conn(), conn.root())?.reply()?;
    if !reply.same_screen {
        tracing::debug!("pointer is on another screen; no logical cursor position");
        return Ok(None);
    }
    Ok(physical_to_global(
        reply.root_x,
        reply.root_y,
        &enumerate(conn)?,
    ))
}

/// Converts an X screen-space physical point into the global logical layout
/// space: the containing output's logical origin plus the output-local
/// physical offset divided by THAT output's scale (never an averaged factor).
/// `None` when no output contains the point.
pub(crate) fn physical_to_global(
    x: i16,
    y: i16,
    monitors: &[MonitoredOutput],
) -> Option<LogicalPoint> {
    let monitor = monitors.iter().find(|monitor| {
        let data = &monitor.data;
        x >= data.x
            && y >= data.y
            && i32::from(x) < i32::from(data.x) + i32::from(data.width)
            && i32::from(y) < i32::from(data.y) + i32::from(data.height)
    })?;
    let local = PhysicalPoint::from_raw(
        i32::from(x) - i32::from(monitor.data.x),
        i32::from(y) - i32::from(monitor.data.y),
    )
    .to_logical(monitor.info.scale);
    Some(LogicalPoint::from_raw(
        monitor.info.logical_rect.x.0 + local.x.0,
        monitor.info.logical_rect.y.0 + local.y.0,
    ))
}

/// Reads the cursor image for compositing, when the server provides XFIXES.
///
/// XFIXES rejects every request sent before its per-connection
/// `QueryVersion` negotiation (`BadRequest`), and each capture worker holds a
/// FRESH connection - the construction-time capability probe negotiated a
/// different one - so the version query runs here again before
/// `GetCursorImage`.
///
/// Degradation ladder (each rung logged, none a capture failure): no XFIXES
/// capability, a failing negotiation, or a failing/empty `GetCursorImage` ->
/// `None`. A broken connection surfaces typed on the subsequent image grab,
/// so the cursor read never fails a capture itself.
pub(crate) fn cursor_snapshot(conn: &X11Connection, caps: X11Caps) -> Option<CursorSnapshot> {
    if caps.xfixes.is_none() {
        tracing::debug!("XFIXES is unavailable; cursor painting is disabled for this capture");
        return None;
    }
    let (major, minor) = XFIXES_CLIENT_VERSION;
    let negotiated = xfixes::query_version(conn.conn(), major, minor)
        .map_err(ReplyError::from)
        .and_then(Cookie::reply);
    if let Err(error) = negotiated {
        tracing::debug!(%error, "XFIXES version negotiation failed; cursor painting is disabled for this capture");
        return None;
    }
    match xfixes::get_cursor_image(conn.conn())
        .map_err(ReplyError::from)
        .and_then(Cookie::reply)
    {
        Ok(reply) => snapshot_from_reply(&reply),
        Err(error) => {
            tracing::debug!(%error, "GetCursorImage failed; cursor painting is disabled for this capture");
            None
        }
    }
}

/// Converts the XFIXES reply into the compositing snapshot: `0xAARRGGBB`
/// native-endian words to byte order R, G, B, A, and the hotspot position to
/// the image's top-left corner. `None` for an empty image.
fn snapshot_from_reply(reply: &GetCursorImageReply) -> Option<CursorSnapshot> {
    if reply.width == 0 || reply.height == 0 {
        return None;
    }
    let pixels = usize::from(reply.width) * usize::from(reply.height);
    if reply.cursor_image.len() < pixels {
        tracing::debug!(
            delivered = reply.cursor_image.len(),
            expected = pixels,
            "GetCursorImage delivered fewer pixels than its geometry; skipping cursor painting"
        );
        return None;
    }
    let mut rgba = Vec::with_capacity(pixels * 4);
    for pixel in reply.cursor_image.iter().take(pixels) {
        rgba.extend_from_slice(&[
            u8::try_from((pixel >> 16) & 0xFF).unwrap_or(0),
            u8::try_from((pixel >> 8) & 0xFF).unwrap_or(0),
            u8::try_from(pixel & 0xFF).unwrap_or(0),
            u8::try_from(pixel >> 24).unwrap_or(0),
        ]);
    }
    Some(CursorSnapshot {
        top_left: PhysicalPoint::from_raw(
            i32::from(reply.x) - i32::from(reply.xhot),
            i32::from(reply.y) - i32::from(reply.yhot),
        ),
        rgba,
        width: u32::from(reply.width),
        height: u32::from(reply.height),
    })
}

/// One output's screen-space `XRGB8888` pixel canvas for cursor compositing
/// (the Wayland crate's `RgbaCanvas` pattern, pinned to the capture format).
#[derive(Debug)]
pub(crate) struct XrgbCanvas<'a> {
    /// The destination pixels, row-major `XRGB8888` (`width * height * 4`).
    pub data: &'a mut [u8],
    /// Destination width in physical pixels.
    pub width: u16,
    /// Destination height in physical pixels.
    pub height: u16,
    /// The output's top-left corner in X screen coordinates.
    pub origin: (i16, i16),
}

/// Alpha-composites the cursor snapshot onto one output's screen-space
/// `XRGB8888` canvas (the orientation the user sees; the native-orientation
/// remap runs afterwards, so the cursor rotates with the output content).
///
/// The cursor is clipped to the canvas bounds, so a cursor hanging off a
/// screen edge - or sitting entirely on another output - composites partially
/// or not at all, never out of bounds. The destination's X byte stays
/// untouched (downstream conversion forces alpha 255 for `XRGB8888`).
pub(crate) fn composite_cursor_xrgb(canvas: XrgbCanvas<'_>, cursor: &CursorSnapshot) {
    let XrgbCanvas {
        data: dest,
        width,
        height,
        origin,
    } = canvas;
    let dest_w = usize::from(width);
    let dest_h = usize::from(height);
    let top_left_x = i64::from(cursor.top_left.x.0) - i64::from(origin.0);
    let top_left_y = i64::from(cursor.top_left.y.0) - i64::from(origin.1);
    let image_w = usize::try_from(cursor.width).unwrap_or(usize::MAX);
    for image_y in 0..cursor.height {
        let Some(dest_y) = offset_index(top_left_y, image_y, dest_h) else {
            continue;
        };
        let row_base = usize::try_from(image_y).unwrap_or(usize::MAX) * image_w * 4;
        for image_x in 0..cursor.width {
            let Some(dest_x) = offset_index(top_left_x, image_x, dest_w) else {
                continue;
            };
            let src_offset = row_base + usize::try_from(image_x).unwrap_or(usize::MAX) * 4;
            let Some(src) = cursor.rgba.get(src_offset..src_offset + 4) else {
                continue;
            };
            let target_offset = (dest_y * dest_w + dest_x) * 4;
            let Some(target) = dest.get_mut(target_offset..target_offset + 4) else {
                continue;
            };
            blend_over_opaque(target, src);
        }
    }
}

/// The destination index for one image pixel along an axis, or `None` when it
/// falls outside `[0, bound)`.
fn offset_index(origin: i64, image_offset: u32, bound: usize) -> Option<usize> {
    let index = origin + i64::from(image_offset);
    if index < 0 {
        return None;
    }
    let index = usize::try_from(index).ok()?;
    (index < bound).then_some(index)
}

/// Source-over blend of one straight-alpha `RGBA` pixel into an opaque `XRGB`
/// destination pixel (destination alpha is defined as 255; the X byte is left
/// alone).
fn blend_over_opaque(dst: &mut [u8], src: &[u8]) {
    let src_a = u16::from(src[3]);
    if src_a == 0 {
        return;
    }
    if src_a == 255 {
        dst[..3].copy_from_slice(&src[..3]);
        return;
    }
    let inv = 255 - src_a;
    for channel in 0..3 {
        let blended = (u16::from(src[channel]) * src_a + u16::from(dst[channel]) * inv) / 255;
        dst[channel] = u8::try_from(blended).unwrap_or(u8::MAX);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    // Fixture scales are exact binary fractions and fixture geometry is
    // integral (geometry-notepad convention).
    #![allow(clippy::float_cmp)]

    use flowshot_core::geometry::{
        Logical, LogicalRect, OutputInfo, PhysicalPx, PhysicalSize, Transform,
    };
    use x11rb::protocol::randr::Rotation;

    use super::*;
    use crate::output::MonitorData;

    fn monitor(
        connector: &str,
        x: i16,
        y: i16,
        width: u16,
        height: u16,
        scale: f64,
    ) -> MonitoredOutput {
        let data = MonitorData {
            connector: connector.to_owned(),
            x,
            y,
            width,
            height,
            mm_width: 0,
            mm_height: 0,
            rotation: Rotation::ROTATE0,
        };
        let info = OutputInfo::new(
            connector,
            connector,
            LogicalRect::new(
                Logical(f64::from(x) / scale),
                Logical(f64::from(y) / scale),
                Logical(f64::from(width) / scale),
                Logical(f64::from(height) / scale),
            ),
            PhysicalSize::new(PhysicalPx(i32::from(width)), PhysicalPx(i32::from(height))),
            scale,
            Transform::Normal,
        )
        .unwrap();
        MonitoredOutput { data, info }
    }

    /// This machine's panel at the derived mm-heuristic scale, plus a scale-1
    /// external monitor to its right.
    fn layout() -> Vec<MonitoredOutput> {
        vec![
            monitor("eDP-1", 0, 0, 2880, 1620, 2.25),
            monitor("DP-1", 2880, 0, 1920, 1080, 1.0),
        ]
    }

    #[test]
    fn physical_point_divides_by_the_containing_outputs_scale() {
        let global = physical_to_global(1440, 810, &layout()).unwrap();
        assert_eq!((global.x.0, global.y.0), (640.0, 360.0));
    }

    #[test]
    fn second_monitor_adds_its_logical_origin_at_its_own_scale() {
        // DP-1's logical origin is its screen x divided by its OWN scale
        // (2880 / 1.0 = 2880) - the per-output derivation output.rs pins.
        // Mixed-scale layouts can leave logical gaps between outputs; the
        // mapping stays self-consistent per output (documented approximate).
        let global = physical_to_global(2980, 50, &layout()).unwrap();
        assert_eq!((global.x.0, global.y.0), (2980.0, 50.0));
    }

    #[test]
    fn origin_and_edges_map_onto_half_open_bounds() {
        let origin = physical_to_global(0, 0, &layout()).unwrap();
        assert_eq!((origin.x.0, origin.y.0), (0.0, 0.0));
        // Last pixel of eDP-1 (half-open: 2880 belongs to DP-1).
        let last = physical_to_global(2879, 1619, &layout()).unwrap();
        assert!((last.x.0 - 1279.5555).abs() < 1e-3, "{last:?}");
        let dp1_first = physical_to_global(2880, 0, &layout()).unwrap();
        assert_eq!((dp1_first.x.0, dp1_first.y.0), (2880.0, 0.0));
    }

    #[test]
    fn point_outside_every_output_is_none() {
        assert!(physical_to_global(10_000, 10_000, &layout()).is_none());
        assert!(physical_to_global(-1, 0, &layout()).is_none());
        // Below the shorter DP-1 but right of eDP-1: inside no output.
        assert!(physical_to_global(3000, 1600, &layout()).is_none());
        assert!(physical_to_global(0, 0, &[]).is_none());
    }

    fn reply(width: u16, height: u16, x: i16, y: i16, xhot: u16, yhot: u16) -> GetCursorImageReply {
        GetCursorImageReply {
            sequence: 0,
            length: 0,
            x,
            y,
            width,
            height,
            xhot,
            yhot,
            cursor_serial: 1,
            cursor_image: vec![0; usize::from(width) * usize::from(height)],
        }
    }

    #[test]
    fn argb_words_convert_to_rgba_bytes() {
        // 0xAARRGGBB: alpha 0x11, red 0x22, green 0x33, blue 0x44.
        let mut reply = reply(1, 1, 0, 0, 0, 0);
        reply.cursor_image = vec![0x1122_3344];
        let snapshot = snapshot_from_reply(&reply).unwrap();
        assert_eq!(snapshot.rgba, [0x22, 0x33, 0x44, 0x11]);
    }

    #[test]
    fn top_left_subtracts_the_hotspot_from_the_position() {
        let reply = reply(24, 24, 100, 200, 3, 7);
        let snapshot = snapshot_from_reply(&reply).unwrap();
        assert_eq!((snapshot.top_left.x.0, snapshot.top_left.y.0), (97, 193));
        assert_eq!((snapshot.width, snapshot.height), (24, 24));
    }

    #[test]
    fn empty_or_short_replies_yield_no_snapshot() {
        assert!(snapshot_from_reply(&reply(0, 0, 0, 0, 0, 0)).is_none());
        let short = GetCursorImageReply {
            cursor_image: vec![0; 3],
            ..reply(2, 2, 0, 0, 0, 0)
        };
        assert!(snapshot_from_reply(&short).is_none());
    }

    fn snapshot(width: u32, height: u32, pixel: [u8; 4], top_left: (i32, i32)) -> CursorSnapshot {
        let mut rgba = Vec::with_capacity(usize::try_from(width * height * 4).unwrap());
        for _ in 0..width * height {
            rgba.extend_from_slice(&pixel);
        }
        CursorSnapshot {
            top_left: PhysicalPoint::from_raw(top_left.0, top_left.1),
            rgba,
            width,
            height,
        }
    }

    /// Composites onto a canvas described by raw parts (test shorthand).
    fn composite(
        dest: &mut [u8],
        width: u16,
        height: u16,
        origin: (i16, i16),
        cursor: &CursorSnapshot,
    ) {
        composite_cursor_xrgb(
            XrgbCanvas {
                data: dest,
                width,
                height,
                origin,
            },
            cursor,
        );
    }

    /// Byte offset of pixel `(x, y)` in a `width`-wide buffer.
    fn offset(x: usize, y: usize, width: usize) -> usize {
        (y * width + x) * 4
    }

    #[test]
    fn opaque_cursor_pixel_replaces_rgb_and_keeps_x() {
        let mut dest = vec![10u8, 20, 30, 99, 0, 0, 0, 0]; // 2x1, X byte 99
        let cursor = snapshot(1, 1, [255, 0, 0, 255], (0, 0));
        composite(&mut dest, 2, 1, (0, 0), &cursor);
        assert_eq!(&dest[..4], &[255, 0, 0, 99], "RGB replaced, X untouched");
        assert_eq!(&dest[4..], &[0, 0, 0, 0], "neighbor untouched");
    }

    #[test]
    fn half_alpha_blends_source_over_opaque_destination() {
        let mut dest = vec![0u8, 0, 0, 0];
        let cursor = snapshot(1, 1, [200, 200, 200, 128], (0, 0));
        composite(&mut dest, 1, 1, (0, 0), &cursor);
        // (200*128 + 0*127)/255 = 100 per channel.
        assert!((95..=105).contains(&dest[0]), "got {}", dest[0]);
    }

    #[test]
    fn transparent_cursor_pixel_leaves_the_destination() {
        let mut dest = vec![10u8, 20, 30, 0];
        let cursor = snapshot(1, 1, [255, 255, 255, 0], (0, 0));
        composite(&mut dest, 1, 1, (0, 0), &cursor);
        assert_eq!(&dest[..3], &[10, 20, 30]);
    }

    #[test]
    fn output_origin_shifts_the_cursor_into_output_space() {
        // Cursor top-left at screen (3000, 50); the output starts at screen
        // (2880, 0), so the cursor lands at output-local (120, 50).
        let width = 200u16;
        let mut dest = vec![0u8; usize::from(width) * 100 * 4];
        let cursor = snapshot(1, 1, [255, 255, 255, 255], (3000, 50));
        composite(&mut dest, width, 100, (2880, 0), &cursor);
        let at = offset(120, 50, usize::from(width));
        assert_eq!(&dest[at..at + 4], &[255, 255, 255, 0]);
        assert_eq!(&dest[..4], &[0, 0, 0, 0], "output-local (0,0) untouched");
    }

    #[test]
    fn cursor_hanging_off_the_edge_clips_without_panic() {
        let mut dest = vec![0u8; 2 * 2 * 4]; // 2x2 output
        let cursor = snapshot(2, 2, [255, 255, 255, 255], (1, 1));
        composite(&mut dest, 2, 2, (0, 0), &cursor);
        let at = offset(1, 1, 2);
        assert_eq!(&dest[at..at + 3], &[255, 255, 255]);
        assert_eq!(&dest[..4], &[0, 0, 0, 0], "only (1,1) is covered");
    }

    #[test]
    fn fully_outside_cursor_changes_nothing() {
        let mut dest = vec![7u8; 2 * 2 * 4];
        let cursor = snapshot(2, 2, [255, 255, 255, 255], (10, 10));
        composite(&mut dest, 2, 2, (0, 0), &cursor);
        assert!(dest.iter().all(|byte| *byte == 7));
        let negative = snapshot(2, 2, [255, 255, 255, 255], (-10, -10));
        composite(&mut dest, 2, 2, (0, 0), &negative);
        assert!(dest.iter().all(|byte| *byte == 7));
    }
}
