//! Region math behind [`SoftwareRenderer`]'s cached frames.
//!
//! # The model
//!
//! A session's frames share almost everything: the capture is frozen, the
//! backdrop quad and the wash over it change only when the selection moves, and
//! the chrome is a few hundred pixels of toolbar, pills, and cursor. Rasterizing
//! the whole window per frame pays for pixels that did not change, so the base —
//! the backdrop quad composited with the wash — is kept in its own framebuffer
//! and each frame restores only the region the chrome touched from it, then
//! repaints the chrome there. A frame's cost tracks the changed region, the way
//! the GPU path's does.
//!
//! # Why the seams cannot show
//!
//! The restore and the base recompute are both restricted to an integer-aligned
//! region, and every operation that touches a sub-region is exact at that
//! region's own edges: the copy is a row memcpy, and the wash is painted as
//! integer-aligned strips, so a pixel is fully inside one strip or fully outside
//! it — no half-covered boundary pixel exists to blend a seam. The wash's real
//! (fractional) edges live inside the strips and keep their antialiasing.

#![expect(
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    reason = "physical-pixel byte offsets and extents from non-negative bounds"
)]

use crate::render::geom::{Point, Rect, Size, f32_from_u32};
use crate::render::list::{Command, PathSegment, Shape, TextCommand};

/// A region of the frame: an integer-aligned box as ((origin x, origin y),
/// (width, height)) in physical px.
pub(crate) type Region = ((u32, u32), (u32, u32));

/// The region a command can paint, inflated by a margin so a glyph's ink, a
/// stroke's cap, or a shadow's blur can never fall outside it: an
/// underestimated box leaves stale pixels in the restored region.
pub(crate) fn command_bounds(command: &Command) -> Option<Rect> {
    let area = match command {
        Command::PopClip => None,
        Command::Fill { shape, color: _ } => shape_bounds(shape),
        Command::Stroke {
            shape,
            width,
            color: _,
        } => expand(shape_bounds(shape), *width),
        Command::Dim {
            bounds,
            cutouts: _,
            color: _,
        } => Some(*bounds),
        Command::Invert { rect } => Some(*rect),
        Command::Image(image) => Some(image.dst),
        Command::Shadow {
            rect,
            radius: _,
            spec,
        } => Some(rect.expand(spec.blur)),
        Command::Text(command) => Some(text_bounds(command)),
        Command::PushClip(clip) => Some(clip.rect),
    };
    area.map(|a| a.expand(16.0))
}

/// The box a shape covers, ignoring stroke width and corner radius.
fn shape_bounds(shape: &Shape) -> Option<Rect> {
    match shape {
        Shape::Rect { rect, radius: _ } => Some(*rect),
        Shape::Ellipse { center, radii } => Some(Rect::from_parts(
            center.x - radii.width,
            center.y - radii.height,
            radii.width * 2.0,
            radii.height * 2.0,
        )),
        Shape::Line { from, to } => Some(Rect::from_parts(
            from.x.min(to.x),
            from.y.min(to.y),
            (from.x - to.x).abs(),
            (from.y - to.y).abs(),
        )),
        Shape::Polyline { points, closed: _ } => bounds_of(points.as_slice()),
        Shape::Path { segments } => bounds_of(path_points(segments.as_slice()).as_slice()),
    }
}

/// The box of a shaped text run, generous in both directions so it holds under
/// either anchor and under wrapping.
fn text_bounds(command: &TextCommand) -> Rect {
    let mut width = command.max_width.unwrap_or(command.font_size);
    for _ in command.text.chars() {
        width += command.font_size;
    }
    let mut height = command.line_height;
    for character in command.text.chars() {
        if character == '\n' {
            height += command.line_height;
        }
    }
    Rect::from_parts(
        command.position.x - width,
        command.position.y - height,
        width * 3.0,
        height * 3.0,
    )
}

/// The bounding box of a point set; `None` when empty.
fn bounds_of(points: &[Point]) -> Option<Rect> {
    let first = points.first()?;
    let (left, top, right, bottom) =
        points
            .iter()
            .fold((first.x, first.y, first.x, first.y), |acc, point| {
                (
                    acc.0.min(point.x),
                    acc.1.min(point.y),
                    acc.2.max(point.x),
                    acc.3.max(point.y),
                )
            });
    Some(Rect::from_parts(left, top, right - left, bottom - top))
}

/// The endpoints of a path's segments.
fn path_points(segments: &[PathSegment]) -> Vec<Point> {
    let mut points = [].to_vec();
    for segment in segments {
        match segment {
            PathSegment::MoveTo(point)
            | PathSegment::LineTo(point)
            | PathSegment::QuadTo(_, point)
            | PathSegment::CubeTo(_, _, point) => points.push(*point),
            PathSegment::Close => {}
        }
    }
    points
}

/// The rect grown on every side; `None` propagates.
pub(crate) fn expand(area: Option<Rect>, amount: f32) -> Option<Rect> {
    area.map(|a| a.expand(amount))
}

/// The integer-aligned box of `rect` clipped to the frame extent. Its edges are
/// whole pixels, which is what keeps every operation restricted to it exact.
pub(crate) fn region_box(rect: Rect, extent: Size) -> Option<Region> {
    let left = rect.origin.x.floor().max(0.0) as u32;
    let top = rect.origin.y.floor().max(0.0) as u32;
    let right = rect.right().ceil().min(extent.width) as u32;
    let bottom = rect.bottom().ceil().min(extent.height) as u32;
    if left >= right || top >= bottom {
        return None;
    }
    Some(((left, top), (right - left, bottom - top)))
}

/// The smallest region covering both.
pub(crate) fn union_region(left: Region, right: Region) -> Region {
    let ((left_x, left_y), (left_w, left_h)) = left;
    let ((right_x, right_y), (right_w, right_h)) = right;
    let origin_x = left_x.min(right_x);
    let origin_y = left_y.min(right_y);
    let right_edge = (left_x + left_w).max(right_x + right_w);
    let bottom_edge = (left_y + left_h).max(right_y + right_h);
    (
        (origin_x, origin_y),
        (right_edge - origin_x, bottom_edge - origin_y),
    )
}

/// The region as a rect in frame coordinates.
pub(crate) fn region_rect(region: Region) -> Rect {
    let ((origin_x, origin_y), (width, height)) = region;
    Rect::from_parts(
        f32_from_u32(origin_x),
        f32_from_u32(origin_y),
        f32_from_u32(width),
        f32_from_u32(height),
    )
}

/// The area two rects share; `None` when they only touch at an edge.
pub(crate) fn intersect_rect(left: Rect, right: Rect) -> Option<Rect> {
    let origin_x = left.origin.x.max(right.origin.x);
    let origin_y = left.origin.y.max(right.origin.y);
    let right_edge = left.right().min(right.right());
    let bottom_edge = left.bottom().min(right.bottom());
    if origin_x >= right_edge || origin_y >= bottom_edge {
        return None;
    }
    Some(Rect::from_parts(
        origin_x,
        origin_y,
        right_edge - origin_x,
        bottom_edge - origin_y,
    ))
}

/// The box covering a rect and an optional box so far.
#[expect(clippy::unnecessary_wraps, reason = "consistent with union_region")]
pub(crate) fn union_rect(area: Option<Rect>, rect: Rect) -> Option<Rect> {
    match area {
        Some(area) => Some(Rect::from_parts(
            area.origin.x.min(rect.origin.x),
            area.origin.y.min(rect.origin.y),
            (area.right().max(rect.right())) - area.origin.x.min(rect.origin.x),
            (area.bottom().max(rect.bottom())) - area.origin.y.min(rect.origin.y),
        )),
        None => Some(rect),
    }
}

/// `outer` minus `holes`, as rects that tile the difference exactly once. Each
/// is painted separately, which is why their own edges must not antialias
/// against each other — they are integer-aligned by construction.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "tiling algorithm preserved for multi-cutout dirty regions"
    )
)]
pub(crate) fn subtract_rects(outer: Rect, holes: &[Rect]) -> Vec<Rect> {
    let mut pieces = [outer].to_vec();
    for hole in holes {
        let mut next = [].to_vec();
        for piece in pieces {
            next.extend(split_into(piece, *hole));
        }
        pieces = next;
    }
    pieces
}

/// One rect minus one rect, as the up-to-four rects tiling the difference; a
/// hole that misses the piece leaves it untouched.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "tiling algorithm preserved for multi-cutout dirty regions"
    )
)]
fn split_into(piece: Rect, hole: Rect) -> Vec<Rect> {
    let left = hole.origin.x.max(piece.origin.x);
    let top = hole.origin.y.max(piece.origin.y);
    let right = hole.right().min(piece.right());
    let bottom = hole.bottom().min(piece.bottom());
    if left >= right || top >= bottom {
        return [piece].to_vec();
    }
    let mut pieces = [].to_vec();
    if piece.origin.x < left {
        pieces.push(Rect::from_parts(
            piece.origin.x,
            piece.origin.y,
            left - piece.origin.x,
            piece.size.height,
        ));
    }
    if right < piece.right() {
        pieces.push(Rect::from_parts(
            right,
            piece.origin.y,
            piece.right() - right,
            piece.size.height,
        ));
    }
    if piece.origin.y < top {
        pieces.push(Rect::from_parts(
            left,
            piece.origin.y,
            right - left,
            top - piece.origin.y,
        ));
    }
    if bottom < piece.bottom() {
        pieces.push(Rect::from_parts(
            left,
            bottom,
            right - left,
            piece.bottom() - bottom,
        ));
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::color::Color;
    use crate::render::geom::Size;
    use crate::render::list::ImageCommand;
    use crate::render::list::TextureId;

    #[test]
    fn region_box_snaps_to_integer_and_clamps_extent() {
        let extent = Size::new(100.0, 100.0);
        let rect = Rect::from_parts(1.2, 3.8, 10.4, 15.1);
        let region = region_box(rect, extent).expect("valid region");
        // left=1.2.floor()=1, top=3.8.floor()=3, right=(11.6).ceil()=12, bottom=(18.9).ceil()=19
        assert_eq!(region, ((1, 3), (11, 16)));

        // Rect outside extent returns None
        let outside = Rect::from_parts(150.0, 150.0, 20.0, 20.0);
        assert!(region_box(outside, extent).is_none());

        // Degenerate rect returns None
        let degenerate = Rect::from_parts(10.0, 10.0, 0.0, 0.0);
        assert!(region_box(degenerate, extent).is_none());
    }

    #[test]
    fn region_rect_converts_back_to_frame_coordinates() {
        let region = ((10, 20), (30, 40));
        let rect = region_rect(region);
        assert_eq!(rect, Rect::from_parts(10.0, 20.0, 30.0, 40.0));
    }

    #[test]
    fn intersect_rect_returns_overlap_or_none() {
        let r1 = Rect::from_parts(10.0, 10.0, 50.0, 50.0);
        let r2 = Rect::from_parts(30.0, 30.0, 50.0, 50.0);
        let inter = intersect_rect(r1, r2).expect("intersection");
        assert_eq!(inter, Rect::from_parts(30.0, 30.0, 30.0, 30.0));

        let disjoint = Rect::from_parts(100.0, 100.0, 20.0, 20.0);
        assert!(intersect_rect(r1, disjoint).is_none());

        let adjacent = Rect::from_parts(60.0, 10.0, 20.0, 50.0);
        assert!(intersect_rect(r1, adjacent).is_none());
    }

    #[test]
    fn union_rect_and_union_region_cover_both() {
        let r1 = Rect::from_parts(10.0, 10.0, 20.0, 20.0);
        let r2 = Rect::from_parts(50.0, 60.0, 10.0, 10.0);
        let u = union_rect(Some(r1), r2).expect("union");
        assert_eq!(u, Rect::from_parts(10.0, 10.0, 50.0, 60.0));

        let reg1 = ((10, 10), (20, 20));
        let reg2 = ((50, 60), (10, 10));
        let u_reg = union_region(reg1, reg2);
        assert_eq!(u_reg, ((10, 10), (50, 60)));
    }

    #[test]
    fn subtract_rects_splits_hole_into_disjoint_tiles() {
        let outer = Rect::from_parts(0.0, 0.0, 100.0, 100.0);
        let hole = Rect::from_parts(20.0, 20.0, 60.0, 60.0);
        let pieces = subtract_rects(outer, &[hole]);
        assert_eq!(
            pieces.len(),
            4,
            "inner hole produces 4 non-overlapping strips"
        );

        // Hole missing outer leaves outer untouched
        let miss = Rect::from_parts(200.0, 200.0, 50.0, 50.0);
        let untouched = subtract_rects(outer, &[miss]);
        assert_eq!(untouched.len(), 1);
        assert_eq!(untouched[0], outer);
    }

    #[test]
    fn command_bounds_inflates_with_margin() {
        let fill = Command::Fill {
            shape: Shape::Rect {
                rect: Rect::from_parts(10.0, 10.0, 20.0, 20.0),
                radius: 0.0,
            },
            color: Color::from_rgba8(255, 0, 0, 255),
        };
        let bounds = command_bounds(&fill).expect("bounds");
        // 10-16 = -6, 10-16 = -6, size = 20+32 = 52
        assert_eq!(bounds, Rect::from_parts(-6.0, -6.0, 52.0, 52.0));

        let img = Command::Image(ImageCommand {
            texture: TextureId::new(1),
            dst: Rect::from_parts(50.0, 50.0, 40.0, 40.0),
            src: None,
            alpha: 1.0,
        });
        assert!(command_bounds(&img).is_some());

        assert!(command_bounds(&Command::PopClip).is_none());
    }
}
