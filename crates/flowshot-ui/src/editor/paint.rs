//! The scene -> display-list paint bridge (plan todo 20).
//!
//! Scene objects and tool previews paint through the renderer-agnostic
//! [`PaintSink`] of `flowshot-core`; [`ListSink`] is the `flowshot-ui`
//! implementation converting scene space (GLOBAL LOGICAL px, f32) into one
//! window's local PHYSICAL px with the output's own scale - the same
//! edge-conversion discipline as the selection paint (never an averaged
//! factor; the #4871 physical-first rule).
//!
//! The object-selection outline is the F27 parity visual: black 3px solid
//! under white 1px dotted (`capturewidget.cpp` object outline). The widths
//! and the Qt-DotLine dash rhythm are BEHAVIOR spec constants (like the
//! selection engine's 3px drag threshold), not theme tokens; the dot
//! approximation is 1px on / 2px off at the inner width.

use flowshot_core::geometry::{Logical, OutputInfo, ToPhysical};
use flowshot_core::scene::{
    Color as SceneColor, PaintSink, Point as ScenePoint, Rect as SceneRect,
};

use crate::render::{
    Color, DisplayList, Point, Rect, Shape, TextCommand, f32_from_f64, f32_from_i32,
};

/// Object-selection outline: outer black stroke width, logical px (F27).
pub const OBJECT_OUTLINE_OUTER: f32 = 3.0;
/// Object-selection outline: inner white dotted stroke width, logical px.
pub const OBJECT_OUTLINE_INNER: f32 = 1.0;
/// Qt `DotLine` dash rhythm at 1px pen width: 1px dot...
pub const DASH_ON: f32 = 1.0;
/// ...2px gap.
pub const DASH_OFF: f32 = 2.0;
/// The selection engine's text line-height ratio (cosmic-text needs an
/// explicit line height; same constant as the HUD text).
const LINE_HEIGHT_RATIO: f32 = 1.2;

/// Converts scene colors into renderer colors (both RGBA; the scene stores
/// 8-bit sRGB, the renderer normalized sRGB).
#[must_use]
pub fn render_color(color: SceneColor) -> Color {
    Color::from_rgba8(color.r, color.g, color.b, color.a)
}

/// Bridges a render-token hex color into the scene's 8-bit color.
#[must_use]
pub fn scene_color_from_hex(hex: &str) -> Option<SceneColor> {
    let color = Color::from_hex_token(hex)?;
    Some(SceneColor::new(
        channel(color.r),
        channel(color.g),
        channel(color.b),
        channel(color.a),
    ))
}

/// Parses the `[editor].draw_color` token; a malformed hex falls back to
/// magenta with an error log (the selection engine's broken-token signal).
pub(super) fn parse_draw_color(hex: &str) -> SceneColor {
    scene_color_from_hex(hex).unwrap_or_else(|| {
        tracing::error!(
            target: "flowshot_ui::editor",
            "draw_color token malformed; painting magenta"
        );
        SceneColor::new(255, 0, 255, 255)
    })
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the channel is clamped to [0, 255] before the cast"
)]
fn channel(value: f32) -> u8 {
    (value * 255.0).round().clamp(0.0, 255.0) as u8
}

/// The [`PaintSink`] -> [`DisplayList`] bridge for one window.
pub(super) struct ListSink<'a> {
    list: &'a mut DisplayList,
    output: &'a OutputInfo,
    font_family: Option<&'a str>,
}

impl<'a> ListSink<'a> {
    pub(super) fn new(
        list: &'a mut DisplayList,
        output: &'a OutputInfo,
        font_family: Option<&'a str>,
    ) -> Self {
        Self {
            list,
            output,
            font_family,
        }
    }

    fn local_point(&self, point: ScenePoint) -> Point {
        Point::new(
            local_x(self.output, f64::from(point.x)),
            local_y(self.output, f64::from(point.y)),
        )
    }

    fn local_rect(&self, rect: SceneRect) -> Rect {
        let x0 = local_x(self.output, f64::from(rect.x));
        let y0 = local_y(self.output, f64::from(rect.y));
        let x1 = local_x(self.output, f64::from(rect.x + rect.width));
        let y1 = local_y(self.output, f64::from(rect.y + rect.height));
        Rect::from_parts(x0, y0, x1 - x0, y1 - y0)
    }

    fn local_len(&self, logical: f32) -> f32 {
        local_len(f64::from(logical), self.output.scale)
    }
}

impl PaintSink for ListSink<'_> {
    fn fill_rect(&mut self, rect: SceneRect, color: SceneColor) {
        self.list.fill(
            Shape::Rect {
                rect: self.local_rect(rect),
                radius: 0.0,
            },
            render_color(color),
        );
    }

    fn stroke_rect(&mut self, rect: SceneRect, color: SceneColor, width: f32) {
        self.list.stroke(
            Shape::Rect {
                rect: self.local_rect(rect),
                radius: 0.0,
            },
            self.local_len(width),
            render_color(color),
        );
    }

    fn fill_ellipse(&mut self, rect: SceneRect, color: SceneColor) {
        let local = self.local_rect(rect);
        self.list.fill(
            Shape::Ellipse {
                center: local.center(),
                radii: crate::render::Size::new(local.size.width / 2.0, local.size.height / 2.0),
            },
            render_color(color),
        );
    }

    fn stroke_ellipse(&mut self, rect: SceneRect, color: SceneColor, width: f32) {
        let local = self.local_rect(rect);
        self.list.stroke(
            Shape::Ellipse {
                center: local.center(),
                radii: crate::render::Size::new(local.size.width / 2.0, local.size.height / 2.0),
            },
            self.local_len(width),
            render_color(color),
        );
    }

    fn draw_line(&mut self, from: ScenePoint, to: ScenePoint, color: SceneColor, width: f32) {
        self.list.stroke(
            Shape::Line {
                from: self.local_point(from),
                to: self.local_point(to),
            },
            self.local_len(width),
            render_color(color),
        );
    }

    fn draw_text(&mut self, position: ScenePoint, text: &str, font_size: f32, color: SceneColor) {
        let size = self.local_len(font_size);
        self.list.text(TextCommand {
            position: self.local_point(position),
            text: text.to_owned(),
            font_size: size,
            line_height: size * LINE_HEIGHT_RATIO,
            color: render_color(color),
            family: self.font_family.map(str::to_owned),
            max_width: None,
        });
    }
}

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

fn local_x(output: &OutputInfo, global: f64) -> f32 {
    let offset = Logical(global - output.logical_rect.x.0);
    f32_from_i32(offset.to_physical(output.scale).0)
}

fn local_y(output: &OutputInfo, global: f64) -> f32 {
    let offset = Logical(global - output.logical_rect.y.0);
    f32_from_i32(offset.to_physical(output.scale).0)
}

fn local_len(logical: f64, scale: f64) -> f32 {
    let factor = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    f32_from_f64(logical * factor)
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
    use flowshot_core::scene::{Scene, TextObject};

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
    fn sink_converts_scene_space_with_the_outputs_own_scale() {
        let out = output(1920.0, 2.0);
        let mut list = DisplayList::new();
        {
            let mut sink = ListSink::new(&mut list, &out, Some("Inter"));
            // Scene x=2020 global logical -> (2020-1920)*2 = 200 local px.
            sink.fill_rect(
                SceneRect::new(2020.0, 50.0, 10.0, 20.0),
                SceneColor::new(1, 2, 3, 4),
            );
            sink.draw_line(
                ScenePoint::new(2020.0, 50.0),
                ScenePoint::new(2030.0, 70.0),
                SceneColor::new(9, 9, 9, 255),
                2.0,
            );
            sink.draw_text(
                ScenePoint::new(2020.0, 50.0),
                "hi",
                8.0,
                SceneColor::new(0, 0, 0, 255),
            );
        }
        let commands: Vec<_> = list.iter().collect();
        assert_eq!(commands.len(), 3);
        let Command::Fill {
            shape: Shape::Rect { rect, .. },
            color,
        } = commands[0]
        else {
            panic!("fill rect");
        };
        assert_eq!(rect.origin, Point::new(200.0, 100.0));
        assert_eq!(rect.size.width, 20.0, "10 logical px at scale 2");
        assert_eq!(*color, Color::from_rgba8(1, 2, 3, 4));
        let Command::Stroke { width, .. } = commands[1] else {
            panic!("line stroke");
        };
        assert_eq!(*width, 4.0, "2 logical px stroke at scale 2");
        let Command::Text(text) = commands[2] else {
            panic!("text");
        };
        assert_eq!(text.font_size, 16.0);
        assert_eq!(text.line_height, 16.0 * LINE_HEIGHT_RATIO);
        assert_eq!(text.family.as_deref(), Some("Inter"));
    }

    #[test]
    fn scene_paints_through_the_sink_in_z_order() {
        let mut scene = Scene::new();
        scene.add_object(Box::new(TextObject::new(
            ScenePoint::new(0.0, 0.0),
            "A".to_owned(),
            10.0,
            SceneColor::new(0, 0, 0, 255),
        )));
        let out = output(0.0, 1.0);
        let mut list = DisplayList::new();
        {
            let mut sink = ListSink::new(&mut list, &out, None);
            scene.paint(&mut sink);
        }
        assert_eq!(list.len(), 1);
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
