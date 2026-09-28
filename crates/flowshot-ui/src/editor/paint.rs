//! The scene -> display-list paint bridge.
//!
//! Scene objects and tool previews paint through the renderer-agnostic
//! [`PaintSink`] of `flowshot-core`; [`ListSink`] is the `flowshot-ui`
//! implementation converting scene space (GLOBAL LOGICAL px, f32) into one
//! window's local PHYSICAL px with the output's own scale - the same
//! edge-conversion discipline as the selection paint (never an averaged
//! factor; the #4871 physical-first rule).
//!
//! The object-selection outline lives in [`super::outline`] (split at the
//! 250-LOC ceiling); the local-conversion helpers here are shared
//! with it.

use flowshot_core::geometry::{Logical, LogicalRect, OutputInfo, ToPhysical};
use flowshot_core::scene::{
    Color as SceneColor, PaintSink, Point as ScenePoint, Rect as SceneRect,
};

use crate::render::{
    Color, DisplayList, Point, Rect, Shape, TextCommand, f32_from_f64, f32_from_i32,
};

/// The selection engine's text line-height ratio (cosmic-text needs an
/// explicit line height; same constant as the HUD text). The text
/// session shares it so the edit preview and the committed paint agree.
pub(super) const LINE_HEIGHT_RATIO: f32 = 1.2;

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
        local_rect(
            self.output,
            LogicalRect::from_raw(
                f64::from(rect.x),
                f64::from(rect.y),
                f64::from(rect.width),
                f64::from(rect.height),
            ),
        )
    }

    fn local_len(&self, logical: f32) -> f32 {
        local_len(f64::from(logical), self.output.scale)
    }

    fn local_points(&self, points: &[ScenePoint]) -> Vec<Point> {
        points
            .iter()
            .map(|point| self.local_point(*point))
            .collect()
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

    fn stroke_rounded_rect(&mut self, rect: SceneRect, radius: f32, color: SceneColor, width: f32) {
        self.list.stroke(
            Shape::Rect {
                rect: self.local_rect(rect),
                radius: self.local_len(radius),
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

    fn stroke_polyline(&mut self, points: &[ScenePoint], color: SceneColor, width: f32) {
        if points.len() < 2 {
            return;
        }
        self.list.stroke(
            Shape::Polyline {
                points: self.local_points(points),
                closed: false,
            },
            self.local_len(width),
            render_color(color),
        );
    }

    fn fill_polygon(&mut self, points: &[ScenePoint], color: SceneColor) {
        if points.len() < 3 {
            return;
        }
        self.list.fill(
            Shape::Polyline {
                points: self.local_points(points),
                closed: true,
            },
            render_color(color),
        );
    }

    fn invert_region(&mut self, rect: SceneRect) {
        self.list.invert(self.local_rect(rect));
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

/// Converts a global-logical rect into this output's local physical px
/// (EDGES converted, never the size - the #4871 physical-first rule).
pub(crate) fn local_rect(output: &OutputInfo, rect: LogicalRect) -> Rect {
    let x0 = local_x(output, rect.x.0);
    let y0 = local_y(output, rect.y.0);
    let x1 = local_x(output, rect.x.0 + rect.width.0);
    let y1 = local_y(output, rect.y.0 + rect.height.0);
    Rect::from_parts(x0, y0, x1 - x0, y1 - y0)
}

pub(crate) fn local_x(output: &OutputInfo, global: f64) -> f32 {
    let offset = Logical(global - output.logical_rect.x.0);
    f32_from_i32(offset.to_physical(output.scale).0)
}

pub(crate) fn local_y(output: &OutputInfo, global: f64) -> f32 {
    let offset = Logical(global - output.logical_rect.y.0);
    f32_from_i32(offset.to_physical(output.scale).0)
}

pub(super) fn local_len(logical: f64, scale: f64) -> f32 {
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
}
