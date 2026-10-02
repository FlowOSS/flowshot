//! The bounded-shape two-point tools: rectangle and ellipse.
//!
//! Both use the F27 diagonal-only adjustment (`rectangletool.cpp` /
//! `circletool.cpp` set ONLY `m_supportsDiagonalAdj`): Ctrl snaps the drag
//! vector to 45deg, which locks the rectangle to a square aspect and the
//! ellipse to a circle - the "Ctrl = aspect lock" / "circle lock" convention.
//!
//! Size semantics (the tool-size dispatch): the ellipse stroke width is the
//! shared `draw_thickness` slot (its `tool_size`); the rectangle's
//! `tool_size` IS the corner radius (`[tools.rectangle].corner_radius`,
//! digits/wheel-adjustable), so its stroke width comes from the persisted
//! `[editor].draw_thickness` config value. F27 wheel-on-rect decision:
//! Flameshot's `drawRectangleSize` is documented as the "size for Rectangle
//! rounded corners" (`flameshot.example.ini`) and its FILLED rect reads
//! that one value for the path radius (the pen it also sets is vestigial
//! under `fillPath`), so the radius IS the rectangle's size semantic -
//! kept here, with the wheel made visible by the size-notifier HUD and the
//! cursor dot, which follows the dispatched size (Flameshot's rect
//! `paintMousePreview` sizes from the same `onSizeChanged` value).

use flowshot_core::geometry::{LogicalPoint, LogicalRect};
use flowshot_core::scene::{
    Color as SceneColor, EllipseObject, PaintSink, Point as ScenePoint, Rect as SceneRect,
    RectObject, ToolObject,
};

use super::super::kind::ToolKind;
use super::super::tool::{EditorContext, Tool};
use super::geometry::{Constrain, TwoPoint, logical, paint_preview_dot};
use crate::render::f32_from_u32;

/// The rectangle outline tool (stroke + `[tools.rectangle].corner_radius`).
#[derive(Debug, Default)]
pub struct RectTool {
    stroke: TwoPoint,
    color: SceneColor,
    radius: f32,
}

impl RectTool {
    fn stroke_width(ctx: &EditorContext<'_>) -> f32 {
        f32_from_u32(ctx.config.editor.draw_thickness)
    }

    fn shape(&self, ctx: &EditorContext<'_>, from: ScenePoint, to: ScenePoint) -> RectObject {
        RectObject::new(
            SceneRect::from_points(from, to),
            self.color,
            Self::stroke_width(ctx),
            false,
        )
        .with_corner_radius(self.radius)
    }
}

impl Tool for RectTool {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Rectangle
    }

    fn draw_start(&mut self, _ctx: &EditorContext<'_>, at: LogicalPoint) {
        self.stroke.start(at);
    }

    fn draw_move(&mut self, _ctx: &EditorContext<'_>, at: LogicalPoint) {
        self.stroke.extend(at);
    }

    fn draw_end(
        &mut self,
        ctx: &EditorContext<'_>,
        at: LogicalPoint,
    ) -> Option<Box<dyn ToolObject>> {
        self.stroke.extend(at);
        let (from, to) = self.stroke.finish(ctx, Constrain::DiagonalOnly)?;
        Some(Box::new(self.shape(ctx, from, to)))
    }

    fn paint(&self, ctx: &EditorContext<'_>, sink: &mut dyn PaintSink) {
        // While drawing, the preview paints the exact committed geometry.
        if let Some((from, to)) = self.stroke.endpoints(ctx, Constrain::DiagonalOnly) {
            self.shape(ctx, from, to).paint(sink);
        } else {
            // The hover dot follows the dispatched size (the corner-radius
            // slot) so wheel/digit adjustments are visible before a drag -
            // the Flameshot rect `paintMousePreview` parity.
            paint_preview_dot(sink, ctx, ctx.color, f32_from_u32(ctx.tool_size));
        }
    }

    fn bounding_rect(&self) -> Option<LogicalRect> {
        self.stroke.raw_bounds().map(logical)
    }

    fn is_valid(&self) -> bool {
        self.stroke.drawing()
    }

    fn on_color_changed(&mut self, color: SceneColor) {
        self.color = color;
    }

    /// The rectangle's dispatched size IS the corner radius (its size slot).
    fn on_size_changed(&mut self, size: u32) {
        self.radius = f32_from_u32(size);
    }
}

/// The ellipse outline tool (shared `draw_thickness` slot; Ctrl = circle).
#[derive(Debug, Default)]
pub struct EllipseTool {
    stroke: TwoPoint,
    color: SceneColor,
    thickness: f32,
}

impl EllipseTool {
    fn shape(&self, from: ScenePoint, to: ScenePoint) -> EllipseObject {
        EllipseObject::new(
            SceneRect::from_points(from, to),
            self.color,
            self.thickness,
            false,
        )
    }
}

impl Tool for EllipseTool {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Circle
    }

    fn draw_start(&mut self, _ctx: &EditorContext<'_>, at: LogicalPoint) {
        self.stroke.start(at);
    }

    fn draw_move(&mut self, _ctx: &EditorContext<'_>, at: LogicalPoint) {
        self.stroke.extend(at);
    }

    fn draw_end(
        &mut self,
        ctx: &EditorContext<'_>,
        at: LogicalPoint,
    ) -> Option<Box<dyn ToolObject>> {
        self.stroke.extend(at);
        let (from, to) = self.stroke.finish(ctx, Constrain::DiagonalOnly)?;
        Some(Box::new(self.shape(from, to)))
    }

    fn paint(&self, ctx: &EditorContext<'_>, sink: &mut dyn PaintSink) {
        if let Some((from, to)) = self.stroke.endpoints(ctx, Constrain::DiagonalOnly) {
            self.shape(from, to).paint(sink);
        } else {
            paint_preview_dot(sink, ctx, ctx.color, self.thickness);
        }
    }

    fn bounding_rect(&self) -> Option<LogicalRect> {
        self.stroke.raw_bounds().map(logical)
    }

    fn is_valid(&self) -> bool {
        self.stroke.drawing()
    }

    fn on_color_changed(&mut self, color: SceneColor) {
        self.color = color;
    }

    fn on_size_changed(&mut self, size: u32) {
        self.thickness = f32_from_u32(size);
    }
}
