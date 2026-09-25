//! The cursor-session event sink plus the pure coordinate and compositing
//! math, kept socket-free so the conversion matrix and the permission-denial
//! decision are unit-testable without a live compositor.
//!
//! # Position space
//!
//! `ext-image-copy-capture-v1` reports a cursor `position` relative to the
//! capture source's top-left corner in POST-transform buffer pixel
//! coordinates (physical). The global logical layout space shared by
//! [`OutputInfo`] is also post-transform, so the conversion divides by the
//! output's scale and adds its logical origin; the output transform cancels
//! (both spaces are already upright) and never enters the math.

use flowshot_capture::CursorEvent;
use flowshot_core::geometry::{LogicalPoint, OutputInfo, PhysicalPoint, ToLogical};
use futures::channel::mpsc::UnboundedSender;

/// Where the long-lived cursor stream forwards events as they arrive.
pub(crate) type CursorSink = UnboundedSender<CursorEvent>;

/// One output's cursor-session state, indexed by its position in
/// [`ActiveCursor`]'s layout order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct CursorSessionState {
    /// A `position` event arrived (source-local physical), or `None` when the
    /// cursor never entered this output - or the compositor withheld position
    /// (a `Hyprland` `PERMISSION_TYPE_CURSOR_POS` denial leaves the session
    /// inert).
    pub position: Option<PhysicalPoint>,
    /// The latest `hotspot` offset inside the cursor image, in physical
    /// pixels.
    pub hotspot: Option<PhysicalPoint>,
    /// The cursor entered this output's capture area.
    pub entered: bool,
}

/// The event sink of the cursor sessions in flight.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ActiveCursor {
    /// Per-output session state, in layout order.
    pub sessions: Vec<CursorSessionState>,
}

impl ActiveCursor {
    /// Resets the sink for `count` freshly created cursor sessions.
    pub(crate) fn begin(&mut self, count: usize) {
        self.sessions = vec![CursorSessionState::default(); count];
    }

    /// The mutable state of the session tagged with `index`.
    pub(crate) fn session_mut(&mut self, index: usize) -> Option<&mut CursorSessionState> {
        self.sessions.get_mut(index)
    }

    /// The first session that reported a position, with its output index.
    pub(crate) fn first_position(&self) -> Option<(usize, PhysicalPoint)> {
        self.sessions
            .iter()
            .enumerate()
            .find_map(|(index, session)| session.position.map(|position| (index, position)))
    }

    /// Whether any session reported a position (the one-shot wait predicate).
    pub(crate) fn any_position(&self) -> bool {
        self.sessions
            .iter()
            .any(|session| session.position.is_some())
    }
}

/// Converts a source-local post-transform physical cursor position into the
/// global logical layout space, honoring the output's scale and logical
/// origin. Total: a non-finite or non-positive scale falls back to 1.0 inside
/// [`ToLogical`], and negative local coordinates (the protocol allows the
/// cursor hotspot outside the buffer) map to points before the origin.
#[must_use]
pub fn source_local_to_global(local: PhysicalPoint, output: &OutputInfo) -> LogicalPoint {
    let local_logical = local.to_logical(output.scale);
    LogicalPoint::from_raw(
        output.logical_rect.x.0 + local_logical.x.0,
        output.logical_rect.y.0 + local_logical.y.0,
    )
}

/// Resolves the one-shot cursor position from the collected session state:
/// the first reported position converted to global logical space, or `None`
/// when no session reported one (the cursor overlaps no output, or the
/// compositor denied cursor-position permission and left every session inert).
pub(crate) fn resolve_cursor_pos(
    cursor: &ActiveCursor,
    layout: &[OutputInfo],
) -> Option<LogicalPoint> {
    let (index, local) = cursor.first_position()?;
    let output = layout.get(index)?;
    Some(source_local_to_global(local, output))
}

/// A captured cursor image: `RGBA8888` pixels plus the hotspot offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorImage {
    /// Row-major `RGBA8888` pixels (`width * height * 4` bytes), converted
    /// from the compositor's negotiated format.
    pub rgba: Vec<u8>,
    /// Image width in physical pixels.
    pub width: u32,
    /// Image height in physical pixels.
    pub height: u32,
    /// The hotspot offset inside the image, in physical pixels: the cursor's
    /// active point relative to the image's top-left corner.
    pub hotspot: PhysicalPoint,
}

impl CursorImage {
    /// The image's top-left corner in destination pixels when the cursor's
    /// hotspot sits at `position`: `position - hotspot`.
    #[must_use]
    pub fn top_left_at(&self, position: PhysicalPoint) -> PhysicalPoint {
        PhysicalPoint::from_raw(
            position.x.0 - self.hotspot.x.0,
            position.y.0 - self.hotspot.y.0,
        )
    }
}

/// An `RGBA8888` destination canvas for client-side cursor compositing.
#[derive(Debug)]
pub struct RgbaCanvas<'a> {
    /// The destination pixels, row-major `RGBA8888` (`width * height * 4`).
    pub data: &'a mut [u8],
    /// Destination width in pixels.
    pub width: u32,
    /// Destination height in pixels.
    pub height: u32,
}

/// Alpha-composites an `RGBA8888` cursor image onto an `RGBA8888` destination
/// at `top_left` (destination pixels), clipping to the destination bounds.
///
/// This is the client-side cursor-painting fallback for backends without a
/// compositor paint-cursors option; the `ext-image-copy-capture-v1` backend
/// paints via the session option instead and does not call this. Straight
/// (non-premultiplied) alpha, source-over. Out-of-bounds image pixels are
/// skipped, so a cursor hanging off a screen edge composites partially
/// without panicking.
pub fn composite_cursor_rgba(canvas: RgbaCanvas<'_>, image: &CursorImage, top_left: PhysicalPoint) {
    let RgbaCanvas {
        data: dest,
        width,
        height,
    } = canvas;
    let (Ok(dest_w), Ok(dest_h)) = (usize::try_from(width), usize::try_from(height)) else {
        return;
    };
    let dest_stride = dest_w * 4;
    let image_w = usize::try_from(image.width).unwrap_or(usize::MAX);
    for image_y in 0..image.height {
        let Some(dest_y) = offset_index(top_left.y.0, image_y, dest_h) else {
            continue;
        };
        let row_base = usize::try_from(image_y).unwrap_or(usize::MAX) * image_w * 4;
        for image_x in 0..image.width {
            let Some(dest_x) = offset_index(top_left.x.0, image_x, dest_w) else {
                continue;
            };
            let src_offset = row_base + usize::try_from(image_x).unwrap_or(usize::MAX) * 4;
            let Some(src) = image.rgba.get(src_offset..src_offset + 4) else {
                continue;
            };
            let target_offset = dest_y * dest_stride + dest_x * 4;
            let Some(target) = dest.get_mut(target_offset..target_offset + 4) else {
                continue;
            };
            blend_over(target, src);
        }
    }
}

/// The destination index for one image pixel along an axis, or `None` when it
/// falls outside `[0, bound)`.
fn offset_index(origin: i32, image_offset: u32, bound: usize) -> Option<usize> {
    let image_offset = i64::from(image_offset);
    let index = i64::from(origin) + image_offset;
    if index < 0 {
        return None;
    }
    let index = usize::try_from(index).ok()?;
    (index < bound).then_some(index)
}

/// Source-over blend of one straight-alpha `RGBA` pixel into `dst`.
fn blend_over(dst: &mut [u8], src: &[u8]) {
    let src_a = u16::from(src[3]);
    if src_a == 0 {
        return;
    }
    if src_a == 255 {
        dst.copy_from_slice(&src[..4]);
        return;
    }
    let inv = 255 - src_a;
    for channel in 0..3 {
        let blended = (u16::from(src[channel]) * src_a + u16::from(dst[channel]) * inv) / 255;
        dst[channel] = u8::try_from(blended).unwrap_or(u8::MAX);
    }
    let dst_a = u16::from(dst[3]);
    let out_a = src_a + (dst_a * inv) / 255;
    dst[3] = u8::try_from(out_a).unwrap_or(u8::MAX);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

    use flowshot_core::geometry::{Logical, LogicalRect, PhysicalPx, PhysicalSize, Transform};

    use super::*;

    fn output(
        connector: &str,
        origin: (f64, f64),
        size: (f64, f64),
        physical: (i32, i32),
        scale: f64,
    ) -> OutputInfo {
        OutputInfo::new(
            connector,
            connector,
            LogicalRect::new(
                Logical(origin.0),
                Logical(origin.1),
                Logical(size.0),
                Logical(size.1),
            ),
            PhysicalSize::new(PhysicalPx(physical.0), PhysicalPx(physical.1)),
            scale,
            Transform::Normal,
        )
        .unwrap()
    }

    /// The live QA layout: HDMI-A-1 1920x1080@(0,0) s1 + DP-3 2560x1440@(1920,0) s1.
    fn qa_layout() -> Vec<OutputInfo> {
        vec![
            output("HDMI-A-1", (0.0, 0.0), (1920.0, 1080.0), (1920, 1080), 1.0),
            output("DP-3", (1920.0, 0.0), (2560.0, 1440.0), (2560, 1440), 1.0),
        ]
    }

    #[test]
    fn scale_one_at_origin_maps_local_straight_to_global() {
        let hdmi = &qa_layout()[0];
        let global = source_local_to_global(PhysicalPoint::from_raw(960, 540), hdmi);
        assert_eq!((global.x.0, global.y.0), (960.0, 540.0));
    }

    #[test]
    fn second_monitor_adds_its_logical_origin() {
        let dp3 = &qa_layout()[1];
        let global = source_local_to_global(PhysicalPoint::from_raw(100, 50), dp3);
        assert_eq!((global.x.0, global.y.0), (2020.0, 50.0));
    }

    #[test]
    fn scale_two_divides_local_before_adding_origin() {
        // A scale-2 output at a non-zero logical origin: local physical
        // (200, 100) is local logical (100, 50), placed at origin (1920, 0).
        let hidpi = output("HIDPI", (1920.0, 0.0), (960.0, 540.0), (1920, 1080), 2.0);
        let global = source_local_to_global(PhysicalPoint::from_raw(200, 100), &hidpi);
        assert_eq!((global.x.0, global.y.0), (2020.0, 50.0));
    }

    #[test]
    fn scale_two_at_origin_halves_the_offset() {
        let hidpi = output("HIDPI", (0.0, 0.0), (960.0, 540.0), (1920, 1080), 2.0);
        let global = source_local_to_global(PhysicalPoint::from_raw(1919, 1079), &hidpi);
        assert_eq!((global.x.0, global.y.0), (959.5, 539.5));
    }

    #[test]
    fn negative_local_coordinates_map_before_the_origin() {
        // The protocol allows the hotspot outside the buffer (negative or
        // beyond the size); the conversion stays total.
        let dp3 = &qa_layout()[1];
        let global = source_local_to_global(PhysicalPoint::from_raw(-20, -5), dp3);
        assert_eq!((global.x.0, global.y.0), (1900.0, -5.0));
    }

    #[test]
    fn local_at_the_buffer_edge_maps_to_the_logical_edge() {
        let hidpi = output("HIDPI", (0.0, 0.0), (960.0, 540.0), (1920, 1080), 2.0);
        let global = source_local_to_global(PhysicalPoint::from_raw(1920, 1080), &hidpi);
        assert_eq!((global.x.0, global.y.0), (960.0, 540.0));
    }

    #[test]
    fn resolve_picks_the_first_reporting_session_and_converts() {
        let layout = qa_layout();
        let mut cursor = ActiveCursor::default();
        cursor.begin(2);
        // Cursor is on DP-3 (index 1); HDMI-A-1 (index 0) never entered.
        cursor.session_mut(1).unwrap().position = Some(PhysicalPoint::from_raw(640, 360));
        let global = resolve_cursor_pos(&cursor, &layout).unwrap();
        assert_eq!((global.x.0, global.y.0), (2560.0, 360.0));
    }

    #[test]
    fn resolve_returns_none_when_no_session_reported_a_position() {
        // The permission-denial / cursor-off-screen path: every session inert.
        let layout = qa_layout();
        let mut cursor = ActiveCursor::default();
        cursor.begin(2);
        cursor.session_mut(0).unwrap().entered = true; // entered but no position
        assert!(!cursor.any_position());
        assert!(resolve_cursor_pos(&cursor, &layout).is_none());
    }

    #[test]
    fn resolve_returns_none_on_an_empty_layout() {
        let cursor = ActiveCursor::default();
        assert!(resolve_cursor_pos(&cursor, &[]).is_none());
    }

    #[test]
    fn first_position_reports_the_lowest_reporting_index() {
        let mut cursor = ActiveCursor::default();
        cursor.begin(3);
        cursor.session_mut(2).unwrap().position = Some(PhysicalPoint::from_raw(1, 1));
        cursor.session_mut(0).unwrap().position = Some(PhysicalPoint::from_raw(9, 9));
        assert_eq!(
            cursor.first_position(),
            Some((0, PhysicalPoint::from_raw(9, 9)))
        );
    }

    fn image(width: u32, height: u32, pixel: [u8; 4]) -> CursorImage {
        let mut rgba = Vec::with_capacity(usize::try_from(width * height * 4).unwrap());
        for _ in 0..width * height {
            rgba.extend_from_slice(&pixel);
        }
        CursorImage {
            rgba,
            width,
            height,
            hotspot: PhysicalPoint::zero(),
        }
    }

    /// Byte offset of pixel `(x, y)` in a `width`-wide `RGBA8888` buffer.
    fn rgba_offset(x: usize, y: usize, width: usize) -> usize {
        (y * width + x) * 4
    }

    /// Composites onto a destination described by raw parts (test shorthand).
    fn composite(
        dest: &mut [u8],
        width: u32,
        height: u32,
        image: &CursorImage,
        top_left: PhysicalPoint,
    ) {
        composite_cursor_rgba(
            RgbaCanvas {
                data: dest,
                width,
                height,
            },
            image,
            top_left,
        );
    }

    #[test]
    fn opaque_cursor_pixel_replaces_the_destination() {
        let mut dest = vec![0u8; 4 * 4 * 4]; // 4x4 transparent black
        let cursor = image(2, 2, [255, 0, 0, 255]);
        composite(&mut dest, 4, 4, &cursor, PhysicalPoint::from_raw(1, 1));
        // Pixel (1,1) is now opaque red.
        let offset = rgba_offset(1, 1, 4);
        assert_eq!(&dest[offset..offset + 4], &[255, 0, 0, 255]);
        // Pixel (0,0) untouched.
        assert_eq!(&dest[0..4], &[0, 0, 0, 0]);
    }

    #[test]
    fn transparent_cursor_pixel_leaves_the_destination() {
        let mut dest = vec![10u8, 20, 30, 255, 0, 0, 0, 0];
        let cursor = image(1, 1, [255, 255, 255, 0]);
        composite(&mut dest, 2, 1, &cursor, PhysicalPoint::from_raw(0, 0));
        assert_eq!(&dest[0..4], &[10, 20, 30, 255]);
    }

    #[test]
    fn half_alpha_blends_source_over_destination() {
        let mut dest = vec![0u8, 0, 0, 255, 0, 0, 0, 0];
        let cursor = image(1, 1, [200, 200, 200, 128]);
        composite(&mut dest, 2, 1, &cursor, PhysicalPoint::from_raw(0, 0));
        // ~ (200*128 + 0*127)/255 = 100 per channel; alpha stays ~255.
        assert!((95..=105).contains(&dest[0]), "got {}", dest[0]);
        assert_eq!(dest[3], 255);
    }

    #[test]
    fn cursor_hanging_off_the_edge_clips_without_panic() {
        let mut dest = vec![0u8; 2 * 2 * 4]; // 2x2
        let cursor = image(2, 2, [255, 255, 255, 255]);
        // Top-left at (1,1): only the (1,1) destination pixel is covered.
        composite(&mut dest, 2, 2, &cursor, PhysicalPoint::from_raw(1, 1));
        let offset = rgba_offset(1, 1, 2);
        assert_eq!(&dest[offset..offset + 4], &[255, 255, 255, 255]);
        assert_eq!(&dest[0..4], &[0, 0, 0, 0]);
    }

    #[test]
    fn fully_outside_cursor_changes_nothing() {
        let mut dest = vec![7u8; 2 * 2 * 4];
        let cursor = image(2, 2, [255, 255, 255, 255]);
        composite(&mut dest, 2, 2, &cursor, PhysicalPoint::from_raw(10, 10));
        assert!(dest.iter().all(|byte| *byte == 7));
    }

    #[test]
    fn top_left_subtracts_the_hotspot() {
        let mut cursor = image(4, 4, [0, 0, 0, 255]);
        cursor.hotspot = PhysicalPoint::from_raw(1, 2);
        let top_left = cursor.top_left_at(PhysicalPoint::from_raw(100, 100));
        assert_eq!((top_left.x.0, top_left.y.0), (99, 98));
    }
}
