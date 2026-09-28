//! The freehand path tools: pencil and marker.
//!
//! Both follow the F27 path-stroke lifecycle (`abstractpathtool.cpp`):
//! points accumulate per motion, the stroke is valid from the second point
//! on, and a click without motion commits NOTHING (the plan's zero-length
//! failure case). The pencil simplifies with Ramer-Douglas-Peucker at
//! commit; the marker is the Flameshot `MarkerTool` BORROW-MODIFIED per the
//! plan: a translucent (alpha ~0.5, [`MARKER_ALPHA`]) chisel-cap stroke at
//! `[tools.marker].size` width - Flameshot's 0.35 multiply blend and +14
//! padding are NOT inherited (normal alpha blending through the renderer's
//! premultiplied pipeline; width is exactly the dispatched size).

use flowshot_core::geometry::{LogicalPoint, LogicalRect};
use flowshot_core::scene::{
    Color as SceneColor, MarkerObject, PaintSink, PencilPath, Point as ScenePoint, ToolObject,
};

use super::super::kind::ToolKind;
use super::super::tool::{EditorContext, Tool};
use super::geometry::{
    Constrain, RDP_EPSILON, TwoPoint, logical, paint_preview_dot, points_bounds, scene_point,
    simplify,
};
use crate::render::f32_from_u32;

/// The marker's translucent blend alpha ("alpha ~0.5";
/// 128/255 = 0.502 - the F27 marker opacity constant).
pub const MARKER_ALPHA: u8 = 128;

/// The freehand pencil stroke (shared `draw_thickness` size slot).
#[derive(Debug, Default)]
pub struct PencilTool {
    points: Vec<ScenePoint>,
    color: SceneColor,
    thickness: f32,
}

impl Tool for PencilTool {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Pencil
    }

    fn draw_start(&mut self, _ctx: &EditorContext<'_>, at: LogicalPoint) {
        self.points = vec![scene_point(at)];
    }

    fn draw_move(&mut self, _ctx: &EditorContext<'_>, at: LogicalPoint) {
        self.points.push(scene_point(at));
    }

    fn draw_end(
        &mut self,
        _ctx: &EditorContext<'_>,
        at: LogicalPoint,
    ) -> Option<Box<dyn ToolObject>> {
        let mut points = std::mem::take(&mut self.points);
        let release = scene_point(at);
        if points.last() != Some(&release) {
            points.push(release);
        }
        if points.len() < 2 {
            return None;
        }
        Some(Box::new(PencilPath::new(
            simplify(&points, RDP_EPSILON),
            self.color,
            self.thickness,
        )))
    }

    fn paint(&self, ctx: &EditorContext<'_>, sink: &mut dyn PaintSink) {
        if self.points.is_empty() {
            paint_preview_dot(sink, ctx, ctx.color, self.thickness);
        } else {
            sink.stroke_polyline(&self.points, self.color, self.thickness);
        }
    }

    fn bounding_rect(&self) -> Option<LogicalRect> {
        points_bounds(&self.points).map(logical)
    }

    fn is_valid(&self) -> bool {
        self.points.len() > 1
    }

    fn on_color_changed(&mut self, color: SceneColor) {
        self.color = color;
    }

    fn on_size_changed(&mut self, size: u32) {
        self.thickness = f32_from_u32(size);
    }
}

/// The translucent highlighter stroke (`[tools.marker].size` slot; Ctrl
/// snaps H/V/45deg like the F27 marker's adjustment flags).
#[derive(Debug, Default)]
pub struct MarkerTool {
    stroke: TwoPoint,
    color: SceneColor,
    width: f32,
}

impl MarkerTool {
    /// The draw color with the marker's translucent alpha applied.
    fn marker_color(&self) -> SceneColor {
        SceneColor::new(self.color.r, self.color.g, self.color.b, MARKER_ALPHA)
    }

    fn shape(&self, ctx: &EditorContext<'_>) -> Option<MarkerObject> {
        let (from, to) = self.stroke.endpoints(ctx, Constrain::OrthogonalDiagonal)?;
        Some(MarkerObject::new(from, to, self.marker_color(), self.width))
    }
}

impl Tool for MarkerTool {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Marker
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
        let (from, to) = self.stroke.finish(ctx, Constrain::OrthogonalDiagonal)?;
        Some(Box::new(MarkerObject::new(
            from,
            to,
            self.marker_color(),
            self.width,
        )))
    }

    fn paint(&self, ctx: &EditorContext<'_>, sink: &mut dyn PaintSink) {
        // While drawing, the preview paints the exact committed geometry.
        if self.stroke.drawing()
            && let Some(shape) = self.shape(ctx)
        {
            shape.paint(sink);
        } else {
            paint_preview_dot(sink, ctx, self.marker_color(), self.width);
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
        self.width = f32_from_u32(size);
    }
}
