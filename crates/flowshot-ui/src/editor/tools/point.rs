//! The plain two-point tools (plan todo 21): line and invert.
//!
//! The line is the F27 `TYPE_DRAWER` stroke (Ctrl snaps H/V/45deg via the
//! shared [`TwoPoint`] constrain). The invert tool is the region-filter
//! tool: it commits an [`InvertObject`] whose paint inverts everything
//! below it non-destructively (the frame pixels are never modified - undo
//! removes the object and the inversion disappears). Flameshot's
//! `InvertTool` has no mouse preview (`paintMousePreview` is a no-op) and no
//! adjustment flags - both honored.

use flowshot_core::geometry::{LogicalPoint, LogicalRect};
use flowshot_core::scene::{
    Color as SceneColor, InvertObject, LineObject, PaintSink, Rect as SceneRect, ToolObject,
};

use super::super::kind::ToolKind;
use super::super::tool::{EditorContext, Tool};
use super::geometry::{Constrain, TwoPoint, logical, paint_preview_dot};
use crate::render::f32_from_u32;

/// The straight two-point line (shared `draw_thickness` size slot).
#[derive(Debug, Default)]
pub struct LineTool {
    stroke: TwoPoint,
    color: SceneColor,
    thickness: f32,
}

impl Tool for LineTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Line
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
        Some(Box::new(LineObject::new(
            from,
            to,
            self.color,
            self.thickness,
        )))
    }

    fn paint(&self, ctx: &EditorContext<'_>, sink: &mut dyn PaintSink) {
        if let Some((from, to)) = self.stroke.endpoints(ctx, Constrain::OrthogonalDiagonal) {
            sink.draw_line(from, to, self.color, self.thickness);
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

/// The region color-inversion tool (plan todo 21; no size semantics - the
/// dispatched thickness slot is ignored).
#[derive(Debug, Default)]
pub struct InvertTool {
    stroke: TwoPoint,
}

impl InvertTool {
    fn region(&self, ctx: &EditorContext<'_>) -> Option<SceneRect> {
        let (from, to) = self.stroke.endpoints(ctx, Constrain::Free)?;
        Some(SceneRect::from_points(from, to))
    }
}

impl Tool for InvertTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Invert
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
        // Zero-length rule: a click without move creates NO object (a
        // zero-area region would invert nothing anyway).
        let (from, to) = self.stroke.finish(ctx, Constrain::Free)?;
        Some(Box::new(InvertObject::new(SceneRect::from_points(
            from, to,
        ))))
    }

    fn paint(&self, ctx: &EditorContext<'_>, sink: &mut dyn PaintSink) {
        // The live preview inverts the drag region exactly as committed.
        if let Some(rect) = self.region(ctx) {
            sink.invert_region(rect);
        }
    }

    fn bounding_rect(&self) -> Option<LogicalRect> {
        self.stroke.raw_bounds().map(logical)
    }

    fn is_valid(&self) -> bool {
        self.stroke.drawing()
    }

    /// The inverted region is never a cursor preview (F27 `InvertTool`
    /// `paintMousePreview` is a no-op).
    fn show_mouse_preview(&self) -> bool {
        false
    }
}
