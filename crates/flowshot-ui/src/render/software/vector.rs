//! Vector geometry for the CPU raster path: the display list's [`Shape`]
//! vocabulary converted to tiny-skia paths, plus the region primitives that
//! paint them.
//!
//! Curves go through tiny-skia's own flattening, so the geometry error is that
//! flattening alone; the GPU path hands lyon exact geometry and multisamples
//! instead. The two agree inside the edge band the cross-rasterizer parity
//! policy masks.

#![expect(
    clippy::cast_possible_truncation,
    reason = "physical-pixel bounds clamped into range before each cast"
)]

use tiny_skia::{FillRule, Mask, Path, PathBuilder, Transform};

use crate::render::color::Color;
use crate::render::geometry::{Point, Rect, for_each_dim_strip};
use crate::render::list::{ClipCommand, PathSegment, Shape};
use crate::render::software::buffer::{ClipMask, SoftwareFramebuffer};

/// The fill rule the shape implies, with its path.
///
/// `None` for geometry that cannot describe a region (an empty polyline).
pub(crate) fn shape_path(shape: &Shape) -> Option<(Path, FillRule)> {
    let mut builder = PathBuilder::new();
    match shape {
        Shape::Rect { rect, radius } => {
            let path = rect_path(rect, *radius)?;
            return Some((path, FillRule::EvenOdd));
        }
        Shape::Ellipse { center, radii } => {
            let path = ellipse_path(*center, (radii.width, radii.height))?;
            return Some((path, FillRule::EvenOdd));
        }
        Shape::Line { from, to } => {
            builder.move_to(from.x, from.y);
            builder.line_to(to.x, to.y);
        }
        Shape::Polyline { points, closed } => {
            let mut iter = points.iter();
            let first = iter.next()?;
            builder.move_to(first.x, first.y);
            for point in iter {
                builder.line_to(point.x, point.y);
            }
            if *closed {
                builder.close();
            }
        }
        Shape::Path { segments } => {
            for segment in segments {
                match *segment {
                    PathSegment::MoveTo(point) => builder.move_to(point.x, point.y),
                    PathSegment::LineTo(point) => builder.line_to(point.x, point.y),
                    PathSegment::QuadTo(control, point) => {
                        builder.quad_to(control.x, control.y, point.x, point.y);
                    }
                    PathSegment::CubeTo(first, second, point) => {
                        builder.cubic_to(first.x, first.y, second.x, second.y, point.x, point.y);
                    }
                    PathSegment::Close => builder.close(),
                }
            }
        }
    }
    Some((builder.finish()?, FillRule::EvenOdd))
}

/// The cubic constant for a quarter-circle arc split: the handle fraction of
/// the radius, shared by the rounded-rect corners and the ellipse split.
pub(crate) const ARC_K: f32 = 0.552_284_7;
/// A rectangle path with `radius` rounded corners, clamped to half the
/// smaller side (the GPU path's contract).
#[must_use]
pub(crate) fn rect_path(rect: &Rect, radius: f32) -> Option<Path> {
    let mut builder = PathBuilder::new();
    let radius = radius
        .max(0.0)
        .min(rect.size.width * 0.5)
        .min(rect.size.height * 0.5);
    if radius <= 0.0 {
        push_rect_subpath(&mut builder, rect);
        return builder.finish();
    }
    let (x, y) = (rect.origin.x, rect.origin.y);
    let (right, bottom) = (rect.right(), rect.bottom());
    // The GPU path's corner is a quadratic whose control point is the sharp
    // corner; a cubic with the two-thirds handles is the same curve exactly.
    let third = radius / 3.0;
    builder.move_to(x + radius, y);
    builder.line_to(right - radius, y);
    builder.cubic_to(right - third, y, right, y + third, right, y + radius);
    builder.line_to(right, bottom - radius);
    builder.cubic_to(
        right,
        bottom - third,
        right - third,
        bottom,
        right - radius,
        bottom,
    );
    builder.line_to(x + radius, bottom);
    builder.cubic_to(x + third, bottom, x, bottom - third, x, bottom - radius);
    builder.line_to(x, y + radius);
    builder.cubic_to(x, y + third, x + third, y, x + radius, y);
    builder.close();
    builder.finish()
}

/// An ellipse path from a center and its radii, approximated by the standard
/// four-arc cubic split.
#[must_use]
pub(crate) fn ellipse_path(center: Point, radii: (f32, f32)) -> Option<Path> {
    let mut builder = PathBuilder::new();
    let (cx, cy) = (center.x, center.y);
    let (rx, ry) = radii;
    let arc = ARC_K;
    let (ox, oy) = (rx * arc, ry * arc);
    builder.move_to(cx - rx, cy);
    builder.cubic_to(cx - rx, cy - oy, cx - ox, cy - ry, cx, cy - ry);
    builder.cubic_to(cx + ox, cy - ry, cx + rx, cy - oy, cx + rx, cy);
    builder.cubic_to(cx + rx, cy + oy, cx + ox, cy + ry, cx, cy + ry);
    builder.cubic_to(cx - ox, cy + ry, cx - rx, cy + oy, cx - rx, cy);
    builder.close();
    builder.finish()
}

/// Appends one axis-aligned rectangle as a closed subpath, so a caller can
/// compose an even-odd region with holes.
pub(crate) fn push_rect_subpath(builder: &mut PathBuilder, rect: &Rect) {
    builder.move_to(rect.origin.x, rect.origin.y);
    builder.line_to(rect.right(), rect.origin.y);
    builder.line_to(rect.right(), rect.bottom());
    builder.line_to(rect.origin.x, rect.bottom());
    builder.close();
}

/// Fills a display-list shape.
pub(crate) fn fill(
    framebuffer: &mut SoftwareFramebuffer,
    clip: &[ClipMask],
    shape: &Shape,
    color: &Color,
) {
    if let Shape::Rect { rect, radius } = shape
        && *radius <= 0.0
    {
        framebuffer.paint_fill_rect(rect, clip, color);
        return;
    }
    let Some((path, rule)) = shape_path(shape) else {
        return;
    };
    framebuffer.paint_fill(&path, rule, clip, color);
}

/// Strokes a display-list shape.
pub(crate) fn stroke(
    framebuffer: &mut SoftwareFramebuffer,
    clip: &[ClipMask],
    shape: &Shape,
    width: f32,
    color: &Color,
) {
    let Some((path, _)) = shape_path(shape) else {
        return;
    };
    framebuffer.paint_stroke(&path, width, clip, color);
}

/// The even-odd dim layer: `bounds` filled with every `cutouts` rect punched
/// out, as one path (the GPU path's single even-odd draw).
#[expect(
    clippy::many_single_char_names,
    reason = "r, g, b, a and x, y, w, h components"
)]
pub(crate) fn dim(
    framebuffer: &mut SoftwareFramebuffer,
    clip: &[ClipMask],
    bounds: &Rect,
    cutouts: &[Rect],
    color: &Color,
) {
    if cutouts.is_empty() {
        if clip.is_empty() && is_integer_rect(bounds) {
            let [r, g, b, a] = super::buffer::straight_linear(color);
            let c8 = tiny_skia::Color::from_rgba8(r, g, b, a)
                .premultiply()
                .to_color_u8();
            let color_pm = [c8.red(), c8.green(), c8.blue(), c8.alpha()];
            let (x, y, w, h) = rect_to_usize(bounds);
            framebuffer.blend_rect_direct(x, y, w, h, color_pm);
        } else {
            framebuffer.paint_fill_rect(bounds, clip, color);
        }
        return;
    }

    if cutouts.len() == 1
        && clip.is_empty()
        && is_integer_rect(bounds)
        && is_integer_rect(&cutouts[0])
    {
        let hole = &cutouts[0];
        let [r, g, b, a] = super::buffer::straight_linear(color);
        let c8 = tiny_skia::Color::from_rgba8(r, g, b, a)
            .premultiply()
            .to_color_u8();
        let color_pm = [c8.red(), c8.green(), c8.blue(), c8.alpha()];

        for_each_dim_strip(bounds, hole, |strip| {
            let (x, y, w, h) = rect_to_usize(&strip);
            framebuffer.blend_rect_direct(x, y, w, h, color_pm);
        });
        return;
    }

    let mut builder = PathBuilder::new();
    push_rect_subpath(&mut builder, bounds);
    for cutout in cutouts {
        push_rect_subpath(&mut builder, cutout);
    }
    if let Some(path) = builder.finish() {
        framebuffer.paint_fill(&path, FillRule::EvenOdd, clip, color);
    }
}

fn is_integer_rect(r: &Rect) -> bool {
    r.origin.x.fract() == 0.0
        && r.origin.y.fract() == 0.0
        && r.size.width.fract() == 0.0
        && r.size.height.fract() == 0.0
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "coordinate bounds"
)]
fn rect_to_usize(r: &Rect) -> (usize, usize, usize, usize) {
    (
        r.origin.x.max(0.0) as usize,
        r.origin.y.max(0.0) as usize,
        r.size.width.max(0.0) as usize,
        r.size.height.max(0.0) as usize,
    )
}

/// The invert tool's region filter over `rect`.
pub(crate) fn invert(framebuffer: &mut SoftwareFramebuffer, clip: &[ClipMask], rect: &Rect) {
    let (width, height) = framebuffer.size();
    let Some(mut mask) = Mask::new(width, height) else {
        return;
    };
    let Some(path) = rect_path(rect, 0.0) else {
        return;
    };
    mask.fill_path(&path, FillRule::Winding, true, Transform::identity());
    let x_start = rect.origin.x.floor().max(0.0) as i32;
    let y_start = rect.origin.y.floor().max(0.0) as i32;
    let x_end = rect.right().ceil().max(0.0) as i32;
    let y_end = rect.bottom().ceil().max(0.0) as i32;
    framebuffer.invert_region(&mask, clip, x_start, y_start, x_end, y_end);
}

/// The rounded-rect clip path for a [`ClipCommand`].
#[must_use]
pub(crate) fn clip_path(command: &ClipCommand) -> Option<Path> {
    rect_path(&command.rect, command.radius)
}
