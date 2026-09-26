//! Shared structured paint recorder for the scene object tests (the
//! `#[cfg(test)] pub(crate) mod test_support` pattern: a bare test-mod-local
//! mock cannot be shared across sibling modules).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::{Color, PaintSink, Point, Rect};

/// One recorded [`PaintSink`] call, structured (not string-formatted) so
/// geometry assertions stay exact.
#[derive(Debug, Clone, PartialEq)]
pub enum Call {
    /// `fill_rect(rect, color)`
    FillRect(Rect, Color),
    /// `stroke_rect(rect, color, width)`
    StrokeRect(Rect, Color, f32),
    /// `stroke_rounded_rect(rect, radius, color, width)`
    StrokeRoundedRect(Rect, f32, Color, f32),
    /// `fill_ellipse(rect, color)`
    FillEllipse(Rect, Color),
    /// `stroke_ellipse(rect, color, width)`
    StrokeEllipse(Rect, Color, f32),
    /// `draw_line(from, to, color, width)`
    Line(Point, Point, Color, f32),
    /// `stroke_polyline(points, color, width)`
    Polyline(Vec<Point>, Color, f32),
    /// `fill_polygon(points, color)`
    Polygon(Vec<Point>, Color),
    /// `invert_region(rect)`
    Invert(Rect),
    /// `draw_text(position, text, font_size, color)`
    Text(Point, String, f32, Color),
}

/// Records every paint call for exact assertions.
#[derive(Debug, Default)]
pub struct RecSink {
    /// The recorded calls, in emission order.
    pub calls: Vec<Call>,
}

impl PaintSink for RecSink {
    fn fill_rect(&mut self, rect: Rect, color: Color) {
        self.calls.push(Call::FillRect(rect, color));
    }
    fn stroke_rect(&mut self, rect: Rect, color: Color, width: f32) {
        self.calls.push(Call::StrokeRect(rect, color, width));
    }
    fn stroke_rounded_rect(&mut self, rect: Rect, radius: f32, color: Color, width: f32) {
        self.calls
            .push(Call::StrokeRoundedRect(rect, radius, color, width));
    }
    fn fill_ellipse(&mut self, rect: Rect, color: Color) {
        self.calls.push(Call::FillEllipse(rect, color));
    }
    fn stroke_ellipse(&mut self, rect: Rect, color: Color, width: f32) {
        self.calls.push(Call::StrokeEllipse(rect, color, width));
    }
    fn draw_line(&mut self, from: Point, to: Point, color: Color, width: f32) {
        self.calls.push(Call::Line(from, to, color, width));
    }
    fn stroke_polyline(&mut self, points: &[Point], color: Color, width: f32) {
        self.calls
            .push(Call::Polyline(points.to_vec(), color, width));
    }
    fn fill_polygon(&mut self, points: &[Point], color: Color) {
        self.calls.push(Call::Polygon(points.to_vec(), color));
    }
    fn invert_region(&mut self, rect: Rect) {
        self.calls.push(Call::Invert(rect));
    }
    fn draw_text(&mut self, position: Point, text: &str, font_size: f32, color: Color) {
        self.calls
            .push(Call::Text(position, text.to_owned(), font_size, color));
    }
}
