//! Heuristic classifier for compositor permission-denial frames.
//!
//! `Hyprland`'s permission system (`ecosystem:enforce_permissions`) does not
//! fail denied captures at the protocol level: it delivers a `ready` frame
//! whose content is a black background with a small centered denial image
//! (see `CScreenshareFrame::render` drawing `m_screencopyDeniedTexture`).
//! Surfacing that frame as a screenshot would silently show the user a black
//! image, so the capture runner classifies delivered frames and maps a
//! denial frame onto [`IccError::PermissionDenied`] ->
//! [`PermissionResult::Denied`] / a typed capture error: black-frame +
//! denied semantics, never a hang.
//!
//! The classifier is deliberately conservative to keep false positives on
//! real content near zero: a denial frame is (1) almost entirely pure black,
//! (2) NOT uniformly black (a blanked screen is legitimate black content,
//! not a denial), and (3) carries its few non-black pixels tightly clustered
//! in the center of the frame (the denial glyph). Real desktops virtually
//! always trip the early exit in the first rows (wallpapers, window
//! decorations, and panels are not 97% pure black) or fail the centering
//! test (a taskbar, clock, or cursor sits away from the exact center).
//!
//! [`PermissionResult::Denied`]: flowshot_capture::PermissionResult::Denied

use flowshot_capture::FrameBuffer;

/// A pixel channel value at or below this is "black" (denial backgrounds
/// are pure black; near-black wallpaper gradients do not qualify).
const BLACK_CHANNEL_MAX: u8 = 8;
/// Percentage of black pixels a denial frame must have (integer math:
/// `black * 100 >= total * 97`).
const BLACK_PERCENT_MIN: u64 = 97;
/// Minimum non-black pixel count: the denial glyph is a visible image, a
/// single stuck pixel or cursor dot is not a denial.
const GLYPH_PIXELS_MIN: usize = 16;

/// Classifies a delivered frame as a compositor permission-denial black
/// frame.
///
/// Scans row-major through the frame's stride, exiting early as soon as the
/// non-black share exceeds the denial bound, so real desktop content costs
/// a few rows of scanning. Malformed buffers (data shorter than the
/// stride geometry claims) classify as `false`: a broken frame is an error
/// elsewhere, never a permission statement.
#[must_use]
pub(crate) fn is_denial_frame(buffer: &FrameBuffer) -> bool {
    let (Ok(width), Ok(height), Ok(stride)) = (
        usize::try_from(buffer.width),
        usize::try_from(buffer.height),
        usize::try_from(buffer.stride),
    ) else {
        return false;
    };
    if width == 0 || height == 0 {
        return false;
    }
    let row_bytes = width.saturating_mul(4);

    let mut total: u64 = 0;
    let mut black: u64 = 0;
    let mut glyph: usize = 0;
    let (mut min_x, mut min_y) = (usize::MAX, usize::MAX);
    let (mut max_x, mut max_y) = (0usize, 0usize);

    for y in 0..height {
        let start = y.saturating_mul(stride);
        let Some(row) = buffer.data.get(start..start.saturating_add(row_bytes)) else {
            return false;
        };
        for x in 0..width {
            let pixel = &row[x * 4..x * 4 + 4];
            total += 1;
            if pixel[0] <= BLACK_CHANNEL_MAX
                && pixel[1] <= BLACK_CHANNEL_MAX
                && pixel[2] <= BLACK_CHANNEL_MAX
            {
                black += 1;
            } else {
                glyph += 1;
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
        let non_black = total - black;
        if non_black * 100 > total * (100 - BLACK_PERCENT_MIN) {
            return false;
        }
    }

    if glyph < GLYPH_PIXELS_MIN {
        return false;
    }
    // The denial glyph is centered: confine the non-black bounding box to
    // the central half of the frame on both axes. (The black-share bound was
    // already enforced by the per-row early exit.)
    let (quarter_x, quarter_y) = (width / 4, height / 4);
    min_x >= quarter_x
        && min_y >= quarter_y
        && max_x < width.saturating_sub(quarter_x)
        && max_y < height.saturating_sub(quarter_y)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use bytes::BytesMut;
    use flowshot_capture::FrameFormat;

    use super::*;

    const WIDTH: u32 = 100;
    const HEIGHT: u32 = 60;

    fn frame_with(painter: impl Fn(&mut [u8], usize, usize)) -> FrameBuffer {
        let mut data = vec![0u8; usize::try_from(WIDTH * HEIGHT * 4).unwrap()];
        for y in 0..usize::try_from(HEIGHT).unwrap() {
            for x in 0..usize::try_from(WIDTH).unwrap() {
                let offset = (y * usize::try_from(WIDTH).unwrap() + x) * 4;
                painter(&mut data[offset..offset + 4], x, y);
            }
        }
        FrameBuffer {
            data: BytesMut::from(data.as_slice()),
            width: WIDTH,
            height: HEIGHT,
            stride: WIDTH * 4,
            format: FrameFormat::Xrgb8888,
        }
    }

    fn paint_block(data: &mut [u8], color: [u8; 4], x0: usize, y0: usize, x1: usize, y1: usize) {
        for y in y0..y1 {
            for x in x0..x1 {
                let offset = (y * usize::try_from(WIDTH).unwrap() + x) * 4;
                data[offset..offset + 4].copy_from_slice(&color);
            }
        }
    }

    #[test]
    fn black_frame_with_a_centered_glyph_is_a_denial() {
        // Hyprland denial shape: pure black + small centered light image.
        let mut data = vec![0u8; usize::try_from(WIDTH * HEIGHT * 4).unwrap()];
        paint_block(&mut data, [200, 30, 30, 255], 44, 26, 56, 34);
        let buffer = FrameBuffer {
            data: BytesMut::from(data.as_slice()),
            width: WIDTH,
            height: HEIGHT,
            stride: WIDTH * 4,
            format: FrameFormat::Xrgb8888,
        };
        assert!(is_denial_frame(&buffer));
    }

    #[test]
    fn uniform_black_is_content_not_denial() {
        // A blanked screen is legitimate black content: no glyph, no denial.
        let buffer = frame_with(|_pixel, _x, _y| {});
        assert!(!is_denial_frame(&buffer));
    }

    #[test]
    fn ordinary_desktop_content_is_not_a_denial() {
        let buffer = frame_with(|pixel, x, y| {
            pixel.copy_from_slice(&[
                u8::try_from(x % 251).unwrap(),
                u8::try_from(y % 199).unwrap(),
                128,
                255,
            ]);
        });
        assert!(!is_denial_frame(&buffer));
    }

    #[test]
    fn black_with_an_off_center_glyph_is_not_a_denial() {
        let mut data = vec![0u8; usize::try_from(WIDTH * HEIGHT * 4).unwrap()];
        // Cursor-sized light block in the top-left corner.
        paint_block(&mut data, [255, 255, 255, 255], 2, 2, 8, 8);
        let buffer = FrameBuffer {
            data: BytesMut::from(data.as_slice()),
            width: WIDTH,
            height: HEIGHT,
            stride: WIDTH * 4,
            format: FrameFormat::Xrgb8888,
        };
        assert!(!is_denial_frame(&buffer));
    }

    #[test]
    fn black_with_a_single_stuck_pixel_is_not_a_denial() {
        let mut data = vec![0u8; usize::try_from(WIDTH * HEIGHT * 4).unwrap()];
        paint_block(&mut data, [255, 255, 255, 255], 50, 30, 51, 31);
        let buffer = FrameBuffer {
            data: BytesMut::from(data.as_slice()),
            width: WIDTH,
            height: HEIGHT,
            stride: WIDTH * 4,
            format: FrameFormat::Xrgb8888,
        };
        assert!(!is_denial_frame(&buffer));
    }

    #[test]
    fn mostly_black_with_wide_spread_content_is_not_a_denial() {
        let mut data = vec![0u8; usize::try_from(WIDTH * HEIGHT * 4).unwrap()];
        // Two small light blocks near opposite edges: centered-glyph test
        // must reject even though the frame is >97% black.
        paint_block(&mut data, [255, 255, 255, 255], 1, 1, 3, 3);
        paint_block(&mut data, [255, 255, 255, 255], 97, 57, 99, 59);
        let buffer = FrameBuffer {
            data: BytesMut::from(data.as_slice()),
            width: WIDTH,
            height: HEIGHT,
            stride: WIDTH * 4,
            format: FrameFormat::Xrgb8888,
        };
        assert!(!is_denial_frame(&buffer));
    }

    #[test]
    fn truncated_buffer_data_classifies_as_not_denial() {
        let buffer = FrameBuffer {
            data: BytesMut::from(&[0u8; 16][..]),
            width: WIDTH,
            height: HEIGHT,
            stride: WIDTH * 4,
            format: FrameFormat::Xrgb8888,
        };
        assert!(!is_denial_frame(&buffer));
    }

    #[test]
    fn near_black_background_above_threshold_is_not_a_denial() {
        // Hyprland's default workspace background (#111111) is dark but not
        // "pure black" (17 > BLACK_CHANNEL_MAX): the frame trips the early
        // exit in the first row even before the glyph tests.
        let buffer = frame_with(|pixel, _x, _y| pixel.copy_from_slice(&[17, 17, 17, 255]));
        assert!(!is_denial_frame(&buffer));
    }
}
