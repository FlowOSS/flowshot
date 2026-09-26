//! The seven shape tools of plan todo 21: pencil, line, arrow, rectangle,
//! ellipse, marker, invert - registered onto a [`ToolRegistry`] by
//! [`register_shape_tools`] (the composition roots: the todo-35 binary and
//! the QA harnesses).
//!
//! Shared mechanics live in [`geometry`]: the F27 two-point stroke with the
//! Ctrl drag conventions (`adjustedVector` clean-room: H/V/45deg for
//! line/arrow/marker, diagonal-only square/circle lock for rect/ellipse),
//! the freehand path stroke with Ramer-Douglas-Peucker simplification
//! (epsilon 0.5px at commit), and the F27 `mousePreviewRect` cursor dot.
//!
//! Every tool: press/drag/release lifecycle with a live preview painting
//! the EXACT committed geometry, commit as one scene object (one undo unit
//! via the framework's `commit_object`), zero-length drags commit nothing,
//! color from `[editor].draw_color` and size from the todo-20 dispatch
//! (shared `draw_thickness`; `[tools.marker].size`; the rectangle's slot IS
//! `[tools.rectangle].corner_radius`).

mod arrow;
mod geometry;
mod path;
mod point;
mod shape;

#[cfg(test)]
mod tests;

pub use arrow::ArrowTool;
pub use geometry::RDP_EPSILON;
pub use path::{MARKER_ALPHA, MarkerTool, PencilTool};
pub use point::{InvertTool, LineTool};
pub use shape::{EllipseTool, RectTool};

use super::kind::ToolKind;
use super::registry::ToolRegistry;

/// Registers all seven todo-21 shape tools on `registry` (idempotent;
/// replaces any prior factory for the same kind).
pub fn register_shape_tools(registry: &mut ToolRegistry) {
    registry.register(ToolKind::Pencil, || Box::new(PencilTool::default()));
    registry.register(ToolKind::Line, || Box::new(LineTool::default()));
    registry.register(ToolKind::Arrow, || Box::new(ArrowTool::default()));
    registry.register(ToolKind::Rectangle, || Box::new(RectTool::default()));
    registry.register(ToolKind::Circle, || Box::new(EllipseTool::default()));
    registry.register(ToolKind::Marker, || Box::new(MarkerTool::default()));
    registry.register(ToolKind::Invert, || Box::new(InvertTool::default()));
}
