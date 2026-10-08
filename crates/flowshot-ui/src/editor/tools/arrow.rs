//! The arrow tool (clean-room from the Flameshot arrow spec).
//!
//! A two-point stroke (Ctrl snaps H/V/45deg - the Flameshot marker/arrow
//! adjustment flags) committing an [`ArrowObject`]: the head geometry scales
//! from the thickness, `[tools.arrow].style` selects the straight or curved
//! (quadratic-notch) head, and `[tools.arrow].reverse` flips the head to the
//! press point. Style and reversal are captured PER OBJECT at commit
//! (BORROW-MODIFIED: Flameshot re-reads `reverseArrow` on every paint,
//! retro-flipping committed arrows; `FlowShot`'s serde persistence requires
//! the value on the object).

use flowshot_core::geometry::{LogicalPoint, LogicalRect};
use flowshot_core::scene::{ArrowObject, Color as SceneColor, PaintSink, ToolObject};

use super::super::kind::ToolKind;
use super::super::tool::{EditorContext, Tool};
use super::geometry::{Constrain, TwoPoint, logical, paint_preview_dot};
use crate::render::f32_from_u32;

/// The arrow annotation tool (shared `draw_thickness` size slot).
#[derive(Debug, Default)]
pub struct ArrowTool {
    stroke: TwoPoint,
    color: SceneColor,
    thickness: f32,
}

impl ArrowTool {
    fn shape(&self, ctx: &EditorContext<'_>) -> Option<ArrowObject> {
        let (from, to) = self.stroke.endpoints(ctx, Constrain::OrthogonalDiagonal)?;
        Some(self.arrow(from, to, ctx))
    }

    fn arrow(
        &self,
        from: flowshot_core::scene::Point,
        to: flowshot_core::scene::Point,
        ctx: &EditorContext<'_>,
    ) -> ArrowObject {
        ArrowObject::new(from, to, self.color, self.thickness)
            .with_style(ctx.config.tools.arrow.style)
            .with_reverse(ctx.config.tools.arrow.reverse)
    }
}

impl Tool for ArrowTool {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Arrow
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
        Some(Box::new(self.arrow(from, to, ctx)))
    }

    fn paint(&self, ctx: &EditorContext<'_>, sink: &mut dyn PaintSink) {
        // While drawing, the preview paints the exact committed geometry
        // (head style and reversal included).
        if self.stroke.drawing()
            && let Some(shape) = self.shape(ctx)
        {
            shape.paint(sink);
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
