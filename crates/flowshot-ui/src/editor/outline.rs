//! The object-selection outline (plan todo 20, F27 parity visual): black 3px
//! solid under white 1px dotted (`capturewidget.cpp` object outline). The
//! widths and the Qt-DotLine dash rhythm are BEHAVIOR spec constants (like
//! the selection engine's 3px drag threshold), not theme tokens; the dot
//! approximation is 1px on / 2px off at the inner width. Split from
//! [`super::paint`] at the 250-LOC ceiling (todo 21); the scene -> local
//! physical conversion helpers stay shared with the [`super::paint::ListSink`].

use flowshot_core::geometry::OutputInfo;
use flowshot_core::scene::Rect as SceneRect;

use crate::render::{Color, DisplayList, Point, Rect, Shape};

use super::paint::{local_len, local_x, local_y};

/// Object-selection outline: outer black stroke width, logical px (F27).
pub const OBJECT_OUTLINE_OUTER: f32 = 3.0;
/// Object-selection outline: inner white dotted stroke width, logical px.
pub const OBJECT_OUTLINE_INNER: f32 = 1.0;
/// Qt `DotLine` dash rhythm at 1px pen width: 1px dot...
pub const DASH_ON: f32 = 1.0;
/// ...2px gap.
pub const DASH_OFF: f32 = 2.0;

/// Appends the F27 object-selection outline (black 3px solid + white 1px
/// dotted) around a scene-space bounding rect.
pub(super) fn append_object_outline(list: &mut DisplayList, output: &OutputInfo, rect: SceneRect) {
    let x0 = local_x(output, f64::from(rect.x));
    let y0 = local_y(output, f64::from(rect.y));
    let x1 = local_x(output, f64::from(rect.x + rect.width));
    let y1 = local_y(output, f64::from(rect.y + rect.height));
    let local = Rect::from_parts(x0, y0, x1 - x0, y1 - y0);
    let black = Color::from_rgba8(0, 0, 0, 255);
    let white = Color::from_rgba8(255, 255, 255, 255);
    list.stroke(
        Shape::Rect {
            rect: local,
            radius: 0.0,
        },
        local_len(f64::from(OBJECT_OUTLINE_OUTER), output.scale),
        black,
    );
    let width = local_len(f64::from(OBJECT_OUTLINE_INNER), output.scale);
    let period = local_len(f64::from(DASH_ON + DASH_OFF), output.scale).max(width * 2.0);
    let dash = local_len(f64::from(DASH_ON), output.scale).max(width);
    for (from, to) in [
        (
            Point::new(local.origin.x, local.origin.y),
            Point::new(local.right(), local.origin.y),
        ),
        (
            Point::new(local.right(), local.origin.y),
            Point::new(local.right(), local.bottom()),
        ),
        (
            Point::new(local.right(), local.bottom()),
            Point::new(local.origin.x, local.bottom()),
        ),
        (
            Point::new(local.origin.x, local.bottom()),
            Point::new(local.origin.x, local.origin.y),
        ),
    ] {
        let (dx, dy) = (to.x - from.x, to.y - from.y);
        let length = (dx * dx + dy * dy).sqrt();
        if length.is_nan() || length <= 0.0 {
            continue;
        }
        let (ux, uy) = (dx / length, dy / length);
        let mut walked = 0.0;
        while walked < length {
            let end = (walked + dash).min(length);
            list.stroke(
                Shape::Line {
                    from: Point::new(from.x + ux * walked, from.y + uy * walked),
                    to: Point::new(from.x + ux * end, from.y + uy * end),
                },
                width,
                white,
            );
            walked += period;
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::float_cmp,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic
    )]

    use flowshot_core::geometry::{LogicalRect, PhysicalSize, Transform};

    use crate::render::Command;

    use super::*;

    fn output(origin_x: f64, scale: f64) -> OutputInfo {
        OutputInfo::new(
            "DP-X",
            "DP-X",
            LogicalRect::from_raw(origin_x, 0.0, 1920.0, 1080.0),
            PhysicalSize::from_raw(1920, 1080),
            scale,
            Transform::Normal,
        )
        .expect("valid fixture output")
    }

    #[test]
    fn object_outline_is_black_solid_plus_white_dots() {
        let out = output(0.0, 1.0);
        let mut list = DisplayList::new();
        append_object_outline(&mut list, &out, SceneRect::new(100.0, 100.0, 30.0, 20.0));
        let commands: Vec<_> = list.iter().collect();
        // First: the black 3px solid rect.
        let Command::Stroke {
            shape: Shape::Rect { rect, .. },
            width,
            color,
        } = commands[0]
        else {
            panic!("outer stroke");
        };
        assert_eq!(*rect, Rect::from_parts(100.0, 100.0, 30.0, 20.0));
        assert_eq!(*width, OBJECT_OUTLINE_OUTER);
        assert_eq!(*color, Color::from_rgba8(0, 0, 0, 255));
        // Rest: white 1px dot segments, all axis-aligned on the rect edges.
        let dots = &commands[1..];
        assert!(!dots.is_empty());
        assert!(dots.iter().all(|command| matches!(
            command,
            Command::Stroke { shape: Shape::Line { .. }, width, color }
                if *width == OBJECT_OUTLINE_INNER && *color == Color::from_rgba8(255, 255, 255, 255)
        )));
        // Perimeter 100px at a 3px rhythm -> ~34 dots (corners clip short).
        assert!((25..=34).contains(&dots.len()), "dot count {}", dots.len());
    }

    #[test]
    fn degenerate_outline_draws_no_dots_but_keeps_the_box() {
        let out = output(0.0, 1.0);
        let mut list = DisplayList::new();
        append_object_outline(&mut list, &out, SceneRect::new(5.0, 5.0, 0.0, 0.0));
        assert_eq!(list.len(), 1, "black box only");
    }
}
